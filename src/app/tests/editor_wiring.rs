//! The piano roll and the drum editor in the app: what the user sees change, and what the audio hears.

use eframe::egui::{Color32, Rect, Shape};

use super::*;
use crate::song::{DrumKit, Event, LaneKind, SamplerSlot};
use crate::theme::channel_color;
use crate::ui::drums::{self, Ctl};
use honeydew_audio::drums::PadSample;

/// A widget of the piano roll, of the step sequencer, of channel 1's drum editor, or of its strip `row`.
fn roll(name: &str) -> egui::Id {
    egui::Id::new("piano_roll").with(name)
}
fn steps(name: &str) -> egui::Id {
    egui::Id::new("step_seq").with(name)
}
fn drum_editor(name: &str) -> egui::Id {
    egui::Id::new(("drum_editor", 1_usize)).with(name)
}
fn strip(row: usize, ctl: Ctl) -> egui::Id {
    drum_editor("drum strips").with((row, ctl))
}

/// The keyboard band at the default panel height, where the drum view has its pads, and the centre of pad `index`
/// there.
const KEYBOARD: egui::Rect = egui::Rect::from_min_max(pos2(0.0, KEYS_TOP), pos2(1170.0, 540.0));
fn pad(index: usize) -> Pos2 {
    pad_in(crate::ui::keyboard_settings::PANEL_H, index)
}

/// The centre of pad `index` under a keyboard panel `panel_u` U tall, once it has its height. On the default 5 U
/// panel only the first row (pads 0–6) is on the screen: the grid keeps its least size and overflows below.
pub(super) fn pad_in(panel_u: f32, index: usize) -> Pos2 {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    let keys = Regions::sized(screen, panel_u * U, 1, &m).keyboard;
    let area = crate::ui::keyboard_settings::pads_rect(keys, &m);
    drums::pad_grid(area, 1170.0, U).centres[index]
}

/// The bottom editors' one close X (`ui::split::close_button`).
pub(super) const EDITOR_CLOSE: &str = "editor_close";
/// The top of a clip editor: the area's top, as it opens fully expanded (`ui::split`).
pub(super) const EDITOR_TOP: f32 = 0.0;

/// The gap between a drum editor's header column and its grid: t = ceil(U/16) px.
const T: f32 = 2.0;

impl Rig {
    /// The rects the last frame filled with `color`.
    /// The rects filled with `color`, groups flattened (the playlist's playhead line is one).
    pub(super) fn rects(&self, color: Color32) -> Vec<Rect> {
        fn walk(shape: &Shape, color: Color32, out: &mut Vec<Rect>) {
            match shape {
                Shape::Rect(r) if r.fill == color => out.push(r.rect),
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, color, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        (self.out.shapes.iter()).for_each(|s| walk(&s.shape, color, &mut out));
        out
    }

    /// The texts the last frame painted inside `area`, top to bottom.
    fn texts_in(&self, area: Rect) -> Vec<String> {
        let mut texts: Vec<(f32, String)> = (self.out.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Text(t) => Some((t.visual_bounding_rect().center(), t.galley.text())),
                _ => None,
            })
            .filter(|(at, _)| area.contains(*at))
            .map(|(at, text)| (at.y, text.to_owned()))
            .collect();
        texts.sort_by(|a, b| a.0.total_cmp(&b.0));
        texts.into_iter().map(|t| t.1).collect()
    }

    /// An editor's playhead: the x of its play box on the ruler ([`r3`]), and whether its line is drawn down the
    /// `grid` widget (the roll's or the step view's).
    pub(super) fn playhead(&self, grid: egui::Id) -> (f32, bool) {
        let (handle, line) = self.editor_playhead(grid);
        (handle.expect("the playhead's handle").0, line.is_some())
    }

    /// An editor's playhead as drawn: its play box's playhead x ([`r3`]) and alpha (`menuFill` at the fade's alpha), if
    /// drawn on the ruler at the editor's top, and its line's x, if drawn down the `grid` widget.
    pub(super) fn editor_playhead(&self, grid: egui::Id) -> (Option<(f32, u8)>, Option<f32>) {
        (
            self.play_box_in(EDITOR_TOP..EDITOR_TOP + RULER),
            self.line_in(grid),
        )
    }

    /// The play box drawn with its centre in the band `ys` (a ruler): the playhead x it shows ([`r3`]) and its alpha
    /// (`menuFill` at the fade's alpha).
    pub(super) fn play_box_in(&self, ys: std::ops::Range<f32>) -> Option<(f32, u8)> {
        use crate::ui::timeline::testing::{play_boxes, playhead_x};
        play_boxes(&self.out.shapes)
            .find(|b| ys.contains(&b.rect.center().y))
            .map(|b| (r3(playhead_x(b.rect, U)), b.fill.a()))
    }

    /// The x of the playhead line (`text`, narrower than U) drawn down the `grid` widget, if any.
    fn line_in(&self, grid: egui::Id) -> Option<f32> {
        let text = self.app.theme.colors.text.to_color32();
        let grid = self.rect_of(grid);
        (self.rects(text).iter())
            .find(|r| r.width() < U && r.y_range() == grid.y_range())
            .map(|r| crate::ui::timeline::testing::line_x(*r, 1.0))
    }

    /// Channel `channel`'s note events.
    fn notes(&self, channel: usize) -> Vec<Event> {
        let clips = &self.app.song.channels[channel].lanes[0].clips;
        clips.iter().flat_map(|c| c.events.clone()).collect()
    }

    fn rect_of(&self, id: egui::Id) -> egui::Rect {
        self.ctx
            .read_response(id)
            .expect("the widget was drawn")
            .rect
    }

    /// A right click, which is also how a long press reaches the widgets.
    pub(super) fn secondary_click(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
    }

    fn kit(&self) -> &DrumKit {
        self.app.song.channels[1].drum_kit.as_ref().expect("pads")
    }

    /// Channel 1's drum hits, as (pad, position).
    fn hits(&self) -> Vec<(u8, u32)> {
        let clips = &self.app.song.channels[1].lanes[0].clips;
        let hit = |e: &Event| match *e {
            Event::Drum { pad, pos, .. } => Some((pad, pos)),
            _ => None,
        };
        clips
            .iter()
            .flat_map(|c| &c.events)
            .filter_map(hit)
            .collect()
    }

    /// Centre of `step` of `row` in a 16-step grid of a new clip's 2 U rows.
    pub(super) fn step(&self, row: usize, step: usize) -> Pos2 {
        let cell = self.app.slots.drum_editor.step.cell_rect(row, step);
        cell.expect("the step view drew").center()
    }

    /// Long-presses `row`'s header and picks the row-menu item at (column, row).
    fn row_menu(&mut self, row: usize, item: (usize, usize)) {
        // Rows move when one is added or removed; egui keeps the rects of the last two frames.
        self.frame(vec![]);
        self.frame(vec![]);
        let header = self.rect_of(egui::Id::new("step_seq").with(("row", row)));
        self.secondary_click(header.center());
        self.frame(vec![]);
        self.click_id(steps("row menu").with(item));
    }
}

/// A Drums channel (1) whose kit has Kick, Snare, Hat and Clap, shown in its drum editor, with the audio
/// started and an empty log.
fn drum_rig() -> Rig {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    let slot = |name: &str| SamplerSlot {
        name: format!("{name} 01"),
        folder: format!("Drum Samples\\{name}"),
        ..SamplerSlot::default()
    };
    // Slot ids 1…4.
    let mut kit = DrumKit::default();
    for name in ["Kick", "Snare", "Hat", "Clap"] {
        kit.insert_slot(kit.slots.len(), slot(name));
    }
    rig.app.song.channels[1].drum_kit = Some(kit);
    // The kit is an edit like any other: the undo history has it.
    rig.app.song_edited(SongEdit::Final, 0);
    rig.click(OUTSIDE);
    rig.edit(1);
    rig.slide_split();
    rig.fold_mixer();
    rig.take();
    rig
}

impl Rig {
    /// Taps the drum editor's current row header, which folds its mixer in or out, and lets the fold finish.
    fn fold_mixer(&mut self) {
        let row = self.app.slots.drum_editor.step.selected;
        self.click_id(egui::Id::new("step_seq").with(("row", row)));
        self.frame(vec![]);
        self.frame(vec![]);
    }
}

#[test]
fn the_drum_mixer_starts_folded_in_and_keeps_its_fold_across_closing_and_reopening() {
    let mut rig = drum_rig();
    rig.fold_mixer();
    // Collapsed: a 3 U header column, no strips and no page tabs.
    let grid = rig.rect_of(steps("grid"));
    assert_eq!(grid.left(), 3.0 * U + T);
    assert!(rig.ctx.read_response(strip(0, Ctl::Level)).is_none());
    assert!(rig.ctx.read_response(drum_editor("pages")).is_none());
    // A tap on another row's header only selects it.
    rig.click_id(egui::Id::new("step_seq").with(("row", 2_usize)));
    rig.frame(vec![]);
    assert_eq!(rig.rect_of(steps("grid")).left(), 3.0 * U + T);
    // Its second tap folds the mixer out: strips and tabs. Closing and reopening the editor keeps it out.
    rig.fold_mixer();
    assert_eq!(rig.rect_of(steps("grid")).left(), 15.0 * U + T);
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.rect_of(steps("grid")).left(), 15.0 * U + T);
    assert!(rig.ctx.read_response(drum_editor("pages")).is_some());
    assert!(rig.ctx.read_response(strip(0, Ctl::Level)).is_some());
}

#[test]
fn the_piano_roll_edits_the_channels_notes_through_the_funnel_and_its_x_goes_back() {
    let mut rig = Rig::with_drums_and_synth();
    rig.click(OUTSIDE);
    rig.edit(2);
    rig.slide_split();
    rig.take();
    // A tap on an empty cell adds a note, in the clip `Rig::edit` opened the roll on.
    rig.click_id(roll("grid"));
    let clips = &rig.app.song.channels[2].lanes[0].clips;
    assert_eq!(clips.iter().map(|c| c.events.len()).sum::<usize>(), 1);
    // It sounds at velocity 1.
    assert_eq!(
        rig.take(),
        ["NoteOn 2 60 127", "ReplaceSong adopting no-resync"]
    );
    rig.click_id(egui::Id::new(EDITOR_CLOSE));
    assert_eq!(rig.app.nav.view, View::Playlist);
}

#[test]
fn each_channel_keeps_its_piano_roll_across_visits() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let snap_on = |rig: &Rig, channel: usize| rig.app.slots.piano_roll.rolls[&channel].snap.on;
    // The playhead knob, which covers the Snap button at beat 0 and takes its touches, at beat 2 (the output, which
    // the first click starts, reports it).
    rig.click(OUTSIDE);
    rig.rec.0.borrow_mut().tick = 256;
    rig.edit(2);
    rig.slide_split();
    // A tap on the snap button turns snap off.
    rig.click_id(roll("snap"));
    assert!(!snap_on(&rig, 2));
    rig.edit(3);
    rig.slide_split();
    assert!(snap_on(&rig, 3));
    rig.click_id(egui::Id::new(EDITOR_CLOSE));
    rig.edit(2);
    rig.slide_split();
    assert!(!snap_on(&rig, 2), "the roll of channel 2 was made anew");
}

#[test]
fn the_drum_editor_toggles_and_swings_steps_through_the_funnel() {
    let mut rig = drum_rig();
    rig.click(rig.step(0, 1));
    assert_eq!(rig.hits(), [(0, 32)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // Half the swing slider's travel swings the offbeat hit once, on the release.
    rig.click_id(steps("swing button"));
    rig.frame(vec![]);
    let slider = rig
        .ctx
        .memory(|m| m.area_rect(steps("swing")))
        .expect("the swing popup is open")
        .center();
    rig.press(slider);
    // The thumb follows the pointer along the 144-point track.
    for dx in [18.0, 36.0, 54.0, 72.0] {
        rig.drag_to(slider + vec2(dx, 0.0));
    }
    assert_eq!(
        rig.take(),
        Vec::<String>::new(),
        "nothing before the release"
    );
    rig.release(slider + vec2(72.0, 0.0));
    rig.frame(vec![]);
    assert_eq!(rig.app.song.channels[1].lanes[0].swing, 0.5);
    assert_eq!(rig.hits(), [(0, 40)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
}

#[test]
fn the_row_menu_swing_cut_and_region_popups_close_with_the_editor() {
    let mut rig = drum_rig();
    let shown = |rig: &Rig, id: egui::Id| rig.ctx.read_response(id).is_some();
    let away_and_back = |rig: &mut Rig| {
        rig.app.navigate(Nav::Show(View::Playlist));
        rig.frame(vec![]);
        rig.app.navigate(Nav::Show(View::DrumEditor(1)));
        rig.slide_split();
        rig.frame(vec![]);
    };
    let menu = steps("row menu").with((0_usize, 0_usize));
    rig.frame(vec![]);
    let header = rig.rect_of(egui::Id::new("step_seq").with(("row", 0)));
    rig.secondary_click(header.center());
    rig.frame(vec![]);
    assert!(shown(&rig, menu), "the row menu opened");
    away_and_back(&mut rig);
    assert!(!shown(&rig, menu), "the row menu came back");

    let slider = steps("swing").with("slider");
    rig.click_id(steps("swing button"));
    rig.frame(vec![]);
    assert!(shown(&rig, slider), "the swing popup opened");
    away_and_back(&mut rig);
    assert!(!shown(&rig, slider), "the swing popup came back");

    rig.app.slots.drum_editor.drums.page = drums::StripPage::Options;
    rig.frame(vec![]);
    let cut = strip(0, Ctl::Cut).with("menu").with((0_usize, 0_usize));
    rig.click_id(strip(0, Ctl::Cut));
    rig.frame(vec![]);
    assert!(shown(&rig, cut), "the cut menu opened");
    away_and_back(&mut rig);
    assert!(!shown(&rig, cut), "the cut menu came back");
    let aux = strip(0, Ctl::Aux).with("menu").with((0_usize, 0_usize));
    rig.click_id(strip(0, Ctl::Aux));
    rig.frame(vec![]);
    assert!(shown(&rig, aux), "the AUX menu opened");
    away_and_back(&mut rig);
    assert!(!shown(&rig, aux), "the AUX menu came back");

    // The ruler's range: its `+`, then the region menu.
    let count = |rig: &Rig, text: &str| {
        (rig.texts_in(Rect::EVERYTHING).iter())
            .filter(|t| *t == text)
            .count()
    };
    let pluses = count(&rig, "+");
    let grid = rig.rect_of(steps("grid"));
    let step = |s: f32| pos2(grid.left() + s * grid.width() / 16.0, EDITOR_TOP + 24.0);
    rig.press(step(2.0));
    rig.next_frame_at(rig.time + 0.31);
    rig.frame(vec![]);
    rig.drag_to(step(4.0));
    rig.release(step(4.0));
    rig.frame(vec![]);
    assert_eq!(count(&rig, "+"), pluses + 1, "the range's `+`");
    away_and_back(&mut rig);
    assert_eq!(count(&rig, "+"), pluses, "the range's `+` came back");
    // Clear of the range's edge handles: a new range.
    rig.press(step(6.0));
    rig.next_frame_at(rig.time + 0.31);
    rig.frame(vec![]);
    rig.drag_to(step(9.0));
    rig.release(step(9.0));
    rig.frame(vec![]);
    rig.click(rig.text_center("+"));
    rig.frame(vec![]);
    assert_eq!(count(&rig, "Insert space"), 1, "the region menu opened");
    away_and_back(&mut rig);
    assert_eq!(count(&rig, "Insert space"), 0, "the region menu came back");
}

#[test]
fn the_expanded_column_is_15_u_in_a_narrow_editor_too() {
    // At 125 % the 1170-point editor is about 29 U wide.
    let mut rig = drum_rig();
    let scale = 125;
    rig.app.settings.ui_scale = scale;
    rig.frame(vec![]);
    rig.frame(vec![]);
    let m = crate::metrics::Metrics::from_pixels_per_point(1.0, scale);
    let u = m.u(1.0);
    let grid = rig.rect_of(steps("grid"));
    assert!(
        grid.right() < 30.0 * u,
        "a narrow editor: {grid:?} at U = {u}"
    );
    assert_eq!(grid.left(), 15.0 * u + m.t());
    let level = rig.rect_of(strip(0, Ctl::Level));
    assert!((level.right() - 15.0 * u).abs() < 1.0, "{level:?}");
}

#[test]
fn the_drum_mixer_column_is_15_u_and_scrolls_with_the_rows() {
    let mut rig = drum_rig();
    let header = |rig: &Rig, row: usize| rig.rect_of(egui::Id::new("step_seq").with(("row", row)));
    // The header column with the mixer expanded: the 3 U header cells, then the strips' page layer, to 15 U.
    let grid = rig.rect_of(steps("grid"));
    assert_eq!(grid.left(), 15.0 * U + T);
    let level = rig.rect_of(strip(0, Ctl::Level));
    assert!((level.right() - 15.0 * U).abs() < 1.0, "{level:?}");
    // A new clip's rows are 2 U apart, each strip on its row.
    for row in 0..4 {
        let (h, level) = (header(&rig, row), rig.rect_of(strip(row, Ctl::Level)));
        // On whole pixels.
        assert_eq!(
            h.top(),
            (grid.top() + row as f32 * 2.0 * U).round(),
            "row {row}"
        );
        assert!(
            h.y_range().contains(level.center().y),
            "row {row}: {level:?}"
        );
    }
    // A drag up on the grid scrolls the headers and the strips together, to the end of the range: the four rows
    // and 1.5 rows of room.
    let range = 5.5 * 2.0 * U - grid.height();
    assert!(range > 0.0, "the rows fit: {grid:?}");
    // Row 3: under the playlist (`ui::split`) the range scrolls row 0 out of the editor.
    let before = (header(&rig, 3), rig.rect_of(strip(3, Ctl::Level)));
    rig.press(grid.center());
    for dy in [-40.0, -80.0, -120.0, -160.0] {
        rig.drag_to(grid.center() + vec2(0.0, dy));
    }
    rig.release(grid.center() + vec2(0.0, -160.0));
    // Pulled past the end (the rubber band), it springs back there.
    for _ in 0..80 {
        rig.frame(vec![]);
    }
    let after = (header(&rig, 3), rig.rect_of(strip(3, Ctl::Level)));
    // The rows lie on whole pixels, the grid's top need not.
    assert!((before.0.top() - after.0.top() - range).abs() <= 0.5);
    assert!((before.1.top() - after.1.top() - range).abs() <= 0.5);
    assert!(rig.hits().is_empty(), "a drag is no tap");
    // The scroll is the clip's: another drum channel's editor opens unscrolled, and coming back finds it.
    rig.app.add_default_drums(0);
    rig.app.song.channels[2].drum_kit = rig.app.song.channels[1].drum_kit.clone();
    for (channel, top) in [(2, before.0.top()), (1, after.0.top())] {
        rig.edit(channel);
        rig.slide_split();
        rig.frame(vec![]);
        assert_eq!(header(&rig, 3).top(), top, "channel {channel}");
    }
}

/// Removing a drum row clears the undo history, the removal's own step included.
#[test]
fn removing_a_drum_row_clears_the_undo_history() {
    let mut rig = drum_rig();
    rig.row_menu(1, (1, 2));
    rig.take();
    assert!(rig.app.history.can_undo());
    rig.row_menu(2, (0, 4));
    rig.frame(vec![]);
    rig.click_id(steps("delete").with((0_usize, 1_usize)));
    assert_eq!(rig.kit().slots.len(), 3);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert!(!rig.app.history.can_undo());
    assert!(!rig.app.history.can_redo());
}

/// Clear and Fill never re-sync, though they take hits away.
#[test]
fn clear_and_fill_take_hits_away_with_no_re_sync() {
    let mut rig = drum_rig();
    rig.click(rig.step(1, 1));
    assert_eq!(rig.hits(), [(1, 32)]);
    rig.take();
    // Fill each 4 step: the hit on step 1 goes.
    rig.row_menu(1, (1, 2));
    assert_eq!(rig.hits(), [(1, 0), (1, 128), (1, 256), (1, 384)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.row_menu(1, (1, 3));
    assert_eq!(rig.hits(), []);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

/// A tap on an empty step never re-syncs, though its hit is an addition and the clip-update rule shrinks the loop it
/// fits; a tap on the hit deletes it, which re-syncs (as Delete).
#[test]
fn a_step_tapped_on_does_not_re_sync_though_it_shrinks_the_loop() {
    let mut rig = drum_rig();
    rig.app.song.channels[1].lanes[0].clips[0].loop_len_beats = 8.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    rig.click(rig.step(0, 1));
    assert_eq!(rig.hits(), [(0, 32)]);
    let clip = &rig.app.song.channels[1].lanes[0].clips[0];
    assert_eq!(clip.loop_len_beats, 4.0);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.click(rig.step(0, 1));
    assert_eq!(rig.hits(), []);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
}

#[test]
fn the_row_menu_edits_the_pattern_and_the_kit_through_the_funnel() {
    let mut rig = drum_rig();
    // Fill each 4 step, on the Snare.
    rig.row_menu(1, (1, 2));
    assert_eq!(rig.hits(), [(1, 0), (1, 128), (1, 256), (1, 384)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // Clone: the Snare's copy is the kit's pad 2 and gets the Snare's hits; the later pads move down.
    rig.row_menu(1, (0, 3));
    let names =
        |rig: &Rig| -> Vec<String> { rig.kit().slots.iter().map(|s| s.name.clone()).collect() };
    assert_eq!(
        names(&rig),
        ["Kick 01", "Snare 01", "Snare 01", "Hat 01", "Clap 01"]
    );
    assert_eq!(rig.hits().len(), 8);
    // Its hits are an addition, as a Paste's: a re-sync.
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // Set Name names the pad (its custom name, from none), not the sample.
    rig.row_menu(2, (0, 6));
    rig.frame(vec![]);
    rig.frame(vec![egui::Event::Text("Rim".into())]);
    rig.frame(vec![key_event(egui::Key::Enter, egui::Modifiers::NONE)]);
    let slot = &rig.kit().slots[2];
    assert_eq!(
        (slot.name.as_str(), slot.file.custom_pad_name.as_str()),
        ("Snare 01", "Rim")
    );
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // Delete asks first; Yes removes the pad and its hits, and the later pads move up.
    rig.row_menu(2, (0, 4));
    rig.frame(vec![]);
    rig.click_id(steps("delete").with((0_usize, 1_usize)));
    assert_eq!(names(&rig), ["Kick 01", "Snare 01", "Hat 01", "Clap 01"]);
    assert_eq!(rig.hits(), [(1, 0), (1, 128), (1, 256), (1, 384)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

#[test]
fn the_strips_edit_the_slot_params_live_and_their_tabs_switch_the_page() {
    let mut rig = drum_rig();
    // The Kick's Level, dragged past its right end.
    let level = rig.rect_of(strip(0, Ctl::Level));
    rig.press(level.center());
    for dx in [10.0, 40.0, level.width()] {
        rig.drag_to(level.center() + vec2(dx, 0.0));
    }
    rig.release(level.center() + vec2(level.width(), 0.0));
    let value = rig.kit().slots[0].params[0];
    assert!(value > 0.8531, "{value}");
    let log = rig.take();
    assert!(
        log.iter().all(|c| c.starts_with("SetParam 1 -1 0 ")),
        "{log:?}"
    );
    assert_eq!(log.last(), Some(&format!("SetParam 1 -1 0 {value:.3}")));
    // The tabs `Mixer`, `Sample`, `Envelope`, `Seq` switch every strip's page.
    let tabs = rig.rect_of(drum_editor("pages"));
    let tab = |i: f32| {
        pos2(
            tabs.left() + tabs.width() * (i + 0.5) / 4.0,
            tabs.center().y,
        )
    };
    rig.click(tab(2.0));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.ctx.read_response(strip(3, Ctl::Atk)).is_some());
    assert!(rig.ctx.read_response(strip(3, Ctl::Level)).is_none());
    // `Cut...` on the Seq page sets the cut group: a song edit.
    rig.click(tab(3.0));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Cut));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Cut).with("menu").with((0_usize, 1_usize)));
    assert_eq!(rig.kit().slots[0].cut_group, 1);
    assert_eq!(rig.take(), ["SetSetting 1 -1 1049 1.000"]);
}

#[test]
fn strip_params_address_their_slot_by_its_id_through_clone_and_delete() {
    // Instance −id is the drum slot with that id, wherever it sits.
    let mut rig = drum_rig();
    // Clone the Snare (its copy is pad 2, id 5), then delete the Kick (yes): the others keep their ids.
    rig.row_menu(1, (0, 3));
    rig.row_menu(0, (0, 4));
    rig.frame(vec![]);
    rig.click_id(steps("delete").with((0_usize, 1_usize)));
    let ids: Vec<u32> = rig.kit().slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [2, 5, 3, 4]);
    rig.take();
    // The copy's Level, on pad 1 now.
    rig.frame(vec![]);
    let level = rig.rect_of(strip(1, Ctl::Level));
    rig.press(level.center());
    rig.drag_to(level.center() + vec2(40.0, 0.0));
    rig.release(level.center() + vec2(40.0, 0.0));
    let value = rig.kit().slots[1].params[0];
    let log = rig.take();
    assert_eq!(
        log.last(),
        Some(&format!("SetParam 1 -5 0 {value:.3}")),
        "{log:?}"
    );
    assert!(
        log.iter().all(|c| c.starts_with("SetParam 1 -5 0 ")),
        "{log:?}"
    );
}

#[test]
fn the_owners_kit_addresses_each_slot_by_its_id_not_its_pad() {
    // 4.10.19 saves slots in id order, each with its pad: the third pad holds slot 8.
    let (mut rig, _) = fresh_song();
    let slots = |rig: &Rig| rig.app.song.channels[2].drum_kit.clone().unwrap().slots;
    let ids: Vec<u32> = slots(&rig).iter().map(|s| s.file.id).collect();
    assert_eq!(ids, [1, 2, 8, 3, 4, 5, 6, 7, 9, 10, 11]);
    rig.edit(2);
    rig.slide_split();
    rig.fold_mixer();
    let strip = egui::Id::new(("drum_editor", 2_usize)).with("drum strips");
    let level = rig.rect_of(strip.with((2_usize, Ctl::Level)));
    rig.press(level.center());
    rig.drag_to(level.center() + vec2(40.0, 0.0));
    rig.release(level.center() + vec2(40.0, 0.0));
    let value = slots(&rig)[2].params[0];
    let log = rig.take();
    assert_eq!(
        log.last(),
        Some(&format!("SetParam 2 -8 0 {value:.3}")),
        "{log:?}"
    );
}

#[test]
fn a_strip_knob_drag_makes_no_undo_step_and_no_rebuild() {
    // No strip control makes an undo step, the bound knobs included (pad params are rack state). The song has changed,
    // and a later edit's undo keeps the drag.
    let mut rig = drum_rig();
    rig.app.files.saved();
    let level = |rig: &Rig| rig.kit().slots[0].params[0];
    let before = level(&rig);
    let steps = rig.app.history.steps();
    let knob = rig.rect_of(strip(0, Ctl::Level));
    rig.press(knob.center());
    for dx in [10.0, 40.0] {
        rig.drag_to(knob.center() + vec2(dx, 0.0));
    }
    rig.release(knob.center() + vec2(40.0, 0.0));
    rig.frame(vec![]);
    let dragged = level(&rig);
    assert!(dragged > before, "{dragged}");
    let log = rig.take();
    assert!(
        !log.is_empty() && log.iter().all(|c| c.starts_with("SetParam 1 -1 0 ")),
        "no rebuild: {log:?}"
    );
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
    // A step tap is an undo step of its own; undoing it keeps the drag.
    rig.click(rig.step(1, 0));
    assert_eq!(rig.hits(), [(1, 0)]);
    assert_eq!(rig.app.history.steps(), steps + 1);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!((rig.hits(), level(&rig)), (vec![], dragged));
}

#[test]
fn an_undone_create_fx_channel_takes_the_pads_target_with_it_after_a_strip_change() {
    // `Create FX Channel`, a Level drag, then its undo: the pad kept the deleted send's id, which the next new channel
    // reuses. Only what a strip change moved goes into the steps before it.
    let mut rig = drum_rig();
    strip_page(&mut rig, drums::StripPage::Options);
    rig.click_id(strip(0, Ctl::Aux));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Aux).with("menu").with((0_usize, 1_usize)));
    rig.frame(vec![]);
    let send = rig.app.song.channels.last().unwrap().id;
    assert_eq!(rig.kit().slots[0].file.send_target, send, "created");
    strip_page(&mut rig, drums::StripPage::Mix);
    let knob = rig.rect_of(strip(0, Ctl::Level));
    rig.press(knob.center());
    rig.drag_to(knob.center() + vec2(400.0, 0.0));
    rig.release(knob.center() + vec2(400.0, 0.0));
    rig.frame(vec![]);
    let level = rig.kit().slots[0].params[0];
    rig.frame(z(egui::Modifiers::NONE));
    assert!(
        rig.app.song.channels.iter().all(|c| c.id != send),
        "the send is gone"
    );
    assert_eq!(
        (
            rig.kit().slots[0].file.send_target,
            rig.kit().slots[0].params[0]
        ),
        (-1, level)
    );
    // The next new channel gets the id back; the pad stays on its own output.
    let end = rig.app.song.channels.len();
    let new = rig
        .app
        .song
        .insert_channel(end, ChannelKind::EffectTrack)
        .unwrap();
    assert_eq!(rig.app.song.channels[new].id, send);
    assert_eq!(rig.kit().slots[0].file.send_target, -1);
}

#[test]
fn the_strips_rack_state_stays_through_an_undo_of_an_earlier_edit_of_its_channel() {
    // Tap a step, drag the strip's Level to 1.0, undo the tap: the Level once came back as 0.8531 (the step before the
    // tap held the channel as it was). Pad params are rack state, with no undo; `Shift` and `AUX` too.
    let mut rig = drum_rig();
    let end = rig.app.song.channels.len();
    let send = rig.app.song.insert_channel(end, ChannelKind::EffectTrack);
    let send = rig.app.song.channels[send.unwrap()].id;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.click(rig.step(1, 0));
    assert_eq!(rig.hits(), [(1, 0)]);
    let knob = rig.rect_of(strip(0, Ctl::Level));
    rig.press(knob.center());
    rig.drag_to(knob.center() + vec2(400.0, 0.0));
    rig.drag_to(knob.center() + vec2(800.0, 0.0));
    rig.release(knob.center() + vec2(800.0, 0.0));
    rig.frame(vec![]);
    strip_page(&mut rig, drums::StripPage::Options);
    let knob = rig.rect_of(strip(0, Ctl::Shift)).center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, 100.0));
    rig.drag_to(knob - vec2(0.0, 216.0));
    rig.release(knob - vec2(0.0, 216.0));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Aux));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Aux).with("menu").with((0_usize, 1_usize)));
    rig.frame(vec![]);
    let strip_state = |rig: &Rig| {
        let slot = &rig.kit().slots[0];
        (slot.params[0], slot.file.beat_offset, slot.file.send_target)
    };
    let set = strip_state(&rig);
    assert!(set.0 == 1.0 && set.1 > 0.0 && set.2 == send, "{set:?}");
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!((rig.hits(), strip_state(&rig)), (vec![], set));
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!((rig.hits(), strip_state(&rig)), (vec![(1, 0)], set));
}

#[test]
fn a_stretch_plays_its_first_8192_frames_at_once_and_the_rest_as_the_worker_streams_them_in() {
    // the first 8192 frames render at once and the pad plays the new, partly filled buffer (a new engine now, which
    // takes over the voices); the rest renders into that buffer on the pick worker, and once it lands the engine is
    // rebuilt with the whole render. No cache walk renders it.
    use honeydew_audio::device::AudioRenderer;
    use honeydew_audio::drums::STREAM_FRAMES;
    let mut rig = stretch_rig();
    let renders = rig.app.audio.samples.renders();
    let held = hold_worker();
    stretch(&mut rig, 0, 200.0);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.kit().slots[0].file.stretch_mode, 1);
    let slot = rig.kit().slots[0].clone();
    let Some(PadSample::Streaming(stream)) = rig.app.audio.samples.pad_sample(&slot) else {
        panic!("the pad plays the stream at once");
    };
    assert_eq!(stream.filled(), STREAM_FRAMES);
    assert!(stream.frames() > 3 * STREAM_FRAMES);
    // A hit plays those frames, then silence where the worker has not been yet, and plays on.
    let key = rig.kit().pad_note(0).expect("a pad note");
    rig.app.audio.send(AudioCommand::NoteOn {
        channel: 1,
        key,
        velocity: 1.0,
    });
    let mut played = vec![0.0; 2 * 16_384];
    (rig.rec.0.borrow_mut().engine.as_mut().unwrap()).render(&mut played, 2);
    // At 48 kHz, the 44.1 kHz buffer's frame 8192 is output frame 8916.
    assert!(played[..2 * 8000].iter().any(|s| s.abs() > 0.01));
    assert!(played[2 * 9000..].iter().all(|&s| s == 0.0));
    assert_eq!(rig.app.slots.drum_editor.stretching(), 1);
    rig.take();
    drop(held);
    assert_eq!(rig.landed(ALL), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
    assert_eq!(stream.filled(), stream.frames());
    let whole = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(whole, Some(PadSample::Whole(_))));
    // The voice took over by the new engine reads the rest at its frame.
    let mut later = vec![0.0; 2 * 4096];
    (rig.rec.0.borrow_mut().engine.as_mut().unwrap()).render(&mut later, 2);
    assert!(later.iter().any(|&s| s != 0.0));
    assert_eq!(rig.app.audio.samples.renders(), renders);
    // A later rebuild renders nothing either: the cache keeps the render.
    rig.app.song_edited(SongEdit::Final, 0);
    assert_eq!(rig.app.audio.samples.renders(), renders);
}

#[test]
fn an_undo_to_a_ratio_the_cache_let_go_streams_it_in_with_no_whole_render_on_the_ui_thread() {
    // A rebuild that misses a render (an undo to an older ratio here; a drumset or song load alike) streams it in as a
    // `Stretch` does, the first frames now and the rest on the worker: no whole buffer renders on the UI thread.
    let mut rig = stretch_rig();
    let ratio = |rig: &mut Rig, ratio: f64| {
        let file = &mut rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[0].file;
        (file.stretch_mode, file.stretch_ratio) = (1, ratio);
        rig.app.song_edited(SongEdit::Final, 0);
    };
    ratio(&mut rig, 2.0);
    rig.landed(ALL);
    // Another ratio: the load of its new engine lets the x2 render go.
    ratio(&mut rig, 0.5);
    rig.landed(ALL);
    let slot = rig.kit().slots[0].clone();
    assert!(matches!(
        rig.app.audio.samples.pad_sample(&slot),
        Some(PadSample::Whole(_))
    ));
    rig.take();
    let held = hold_worker();
    rig.frame(z(egui::Modifiers::NONE));
    let slot = rig.kit().slots[0].clone();
    assert_eq!(slot.file.stretch_ratio, 2.0, "undone");
    assert!(rig.take().contains(&"ReplaceSong adopting".to_owned()));
    let streaming = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(streaming, Some(PadSample::Streaming(s)) if s.filled() < s.frames()));
    assert_eq!(rig.app.audio.samples.renders(), 0, "no whole buffer here");
    drop(held);
    assert_eq!(rig.landed(ALL), ["ReplaceSong adopting no-resync"]);
    let whole = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(whole, Some(PadSample::Whole(_))));
    assert_eq!(rig.app.audio.samples.renders(), 0);
}

#[test]
fn a_song_loads_stretch_streams_in_and_lands_with_no_drum_editor_shown() {
    // A song load's `STR1` render streams in like a `Stretch`; the chrome pass (`after_chrome`) lands it every frame,
    // whatever view shows.
    let mut rig = Rig::new();
    rig.click(OUTSIDE);
    rig.app.add_default_drums(0);
    let mut kit = DrumKit::default();
    kit.insert_slot(0, SamplerSlot::default());
    let slot = &mut kit.slots[0];
    slot.folder = "Built-in/Kick".into();
    (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 2.0);
    rig.app.song.channels[1].drum_kit = Some(kit);
    let held = hold_worker();
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::Playlist);
    let slot = rig.kit().slots[0].clone();
    let streaming = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(streaming, Some(PadSample::Streaming(s)) if s.filled() < s.frames()));
    assert_eq!(rig.app.slots.drum_editor.stretching(), 1);
    rig.take();
    drop(held);
    assert_eq!(rig.landed(ALL), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.app.nav.view, View::Playlist);
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
    let whole = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(whole, Some(PadSample::Whole(_))));
}

#[test]
fn a_strip_drag_tells_the_history_once_at_its_release_or_a_focus_loss() {
    let mut rig = drum_rig();
    let calls = crate::history::rack_state_calls;
    let before = calls();
    let knob = rig.rect_of(strip(0, Ctl::Level));
    rig.press(knob.center());
    for dx in [10.0, 20.0, 30.0, 40.0] {
        rig.drag_to(knob.center() + vec2(dx, 0.0));
    }
    assert_eq!(calls(), before, "not while held");
    rig.release(knob.center() + vec2(40.0, 0.0));
    rig.frame(vec![]);
    assert_eq!(calls(), before + 1);
    // Held over a focus loss (the release may never come): told on that frame.
    rig.press(knob.center());
    rig.drag_to(knob.center() + vec2(-30.0, 0.0));
    assert_eq!(calls(), before + 1);
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    assert_eq!(calls(), before + 2);
}

#[test]
fn a_second_pad_on_the_same_file_and_ratio_shares_the_render_and_rebuilds_at_once() {
    // A buffer with the same file, ratio and pitch that is already loaded is shared.
    let mut rig = stretch_rig();
    let slots = &mut rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots;
    slots[1].folder = "Built-in/Kick".into();
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    stretch(&mut rig, 0, 200.0);
    assert_eq!(
        rig.landed(ALL),
        ["ReplaceSong adopting no-resync"; 2],
        "the stream at once, the whole render once it lands"
    );
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
    stretch(&mut rig, 1, 200.0);
    let (a, b) = (&rig.kit().slots[0].file, &rig.kit().slots[1].file);
    assert_eq!(a.stretch_ratio, b.stretch_ratio);
    assert_eq!(
        rig.app.slots.drum_editor.stretching(),
        0,
        "nothing to render"
    );
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
}

/// [`drum_rig`] with the Kick and Snare on built-in one-shots, decoded by a rebuild, on the `Sample` page; an empty log.
fn stretch_rig() -> Rig {
    let mut rig = drum_rig();
    let slots = &mut rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots;
    (slots[0].folder, slots[1].folder) = ("Built-in/Kick".into(), "Built-in/Snare".into());
    rig.app.song_edited(SongEdit::Final, 0);
    strip_page(&mut rig, drums::StripPage::Pitch);
    rig.take();
    rig
}

/// Drags strip `row`'s Stretch knob `dy` points up and lets go.
fn stretch(rig: &mut Rig, row: usize, dy: f32) {
    let knob = rig.rect_of(strip(row, Ctl::Stretch)).center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, dy / 2.0));
    rig.drag_to(knob - vec2(0.0, dy));
    rig.release(knob - vec2(0.0, dy));
}

/// Holds the pick worker's jobs started from now on until the guard drops.
fn hold_worker() -> std::sync::MutexGuard<'static, ()> {
    crate::ui::views::preset_picker::WORKER_HOLD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Every stretch render [`Rig::landed`] waits for.
const ALL: usize = usize::MAX;

#[test]
fn two_pads_stretching_on_the_worker_rebuild_once_both_have_landed() {
    let mut rig = stretch_rig();
    let first = hold_worker();
    stretch(&mut rig, 0, 200.0);
    let second = hold_worker();
    stretch(&mut rig, 1, 100.0);
    assert_eq!(rig.app.slots.drum_editor.stretching(), 2);
    rig.frame(vec![]);
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting no-resync"; 2],
        "each stream at once"
    );
    // The Kick's render lands into the cache: no rebuild while the Snare's still renders.
    drop(first);
    assert_eq!(rig.landed(1), Vec::<String>::new());
    assert!(rig.app.audio.samples.slot(&rig.kit().slots[0]).is_some());
    assert_eq!(rig.app.slots.drum_editor.stretching(), 1);
    drop(second);
    assert_eq!(rig.landed(ALL), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
    for row in [0, 1] {
        assert!(rig.app.audio.samples.slot(&rig.kit().slots[row]).is_some());
    }
}

#[test]
fn a_stretch_render_that_lands_after_the_ratio_moved_on_is_dropped() {
    let mut rig = stretch_rig();
    let held = hold_worker();
    stretch(&mut rig, 0, 200.0);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // `Reset` back to 1: nothing to render, a rebuild at once.
    rig.secondary_click(rig.rect_of(strip(0, Ctl::Stretch)).center());
    rig.frame(vec![]);
    rig.click(rig.text_center("Reset"));
    assert_eq!(rig.kit().slots[0].file.stretch_ratio, 1.0);
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    drop(held);
    assert_eq!(rig.landed(ALL), Vec::<String>::new());
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
}

#[test]
fn an_edit_and_its_undo_while_a_stretch_renders_keep_the_stretch() {
    // The new ratio is rack state as soon as the finger lifts: a step tap's undo step leads back to it. Neither the
    // tap's nor the undo's new engine renders it again on the UI thread: it plays the stream.
    let mut rig = stretch_rig();
    assert_eq!(rig.app.audio.samples.renders(), 0);
    let held = hold_worker();
    stretch(&mut rig, 0, 200.0);
    let ratio = rig.kit().slots[0].file.stretch_ratio;
    assert!(ratio > 1.0);
    rig.click(rig.step(1, 0));
    assert_eq!(rig.hits(), [(1, 0)]);
    rig.frame(z(egui::Modifiers::NONE));
    assert!(rig.hits().is_empty());
    assert_eq!(rig.kit().slots[0].file.stretch_ratio, ratio);
    assert_eq!(rig.app.audio.samples.renders(), 0, "none on the UI thread");
    let slot = rig.kit().slots[0].clone();
    let streaming = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(streaming, Some(PadSample::Streaming(_))));
    drop(held);
    rig.landed(ALL);
    assert_eq!(rig.app.slots.drum_editor.stretching(), 0);
    assert_eq!(rig.kit().slots[0].file.stretch_ratio, ratio);
    let whole = rig.app.audio.samples.pad_sample(&slot);
    assert!(matches!(whole, Some(PadSample::Whole(_))));
    assert_eq!(rig.app.audio.samples.renders(), 0);
}

/// Shows strip page `page` in the drum editor.
fn strip_page(rig: &mut Rig, page: drums::StripPage) {
    rig.app.slots.drum_editor.drums.page = page;
    rig.frame(vec![]);
    rig.frame(vec![]);
}

#[test]
fn rev_and_poly_reach_the_playing_kit_live_with_no_undo_step() {
    // `REV` toggles MAIN@1036 and `Poly` MAIN@1050, with no re-render (no new engine) and no undo step. Neither sets
    // The song-changed flag.
    let mut rig = drum_rig();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    strip_page(&mut rig, drums::StripPage::Pitch);
    rig.take();
    rig.click_id(strip(1, Ctl::Rev));
    assert!(rig.kit().slots[1].file.reverse);
    assert_eq!(rig.take(), ["SetSetting 1 -2 1036 1.000"]);
    strip_page(&mut rig, drums::StripPage::Options);
    rig.click_id(strip(1, Ctl::Poly));
    assert!(!rig.kit().slots[1].file.overlapping_voices);
    assert_eq!(rig.take(), ["SetSetting 1 -2 1050 0.000"]);
    assert_eq!(rig.app.history.steps(), steps);
    assert!(!rig.app.files.modified());
    // A later edit's undo keeps them.
    rig.click(rig.step(1, 0));
    rig.frame(z(egui::Modifiers::NONE));
    let file = &rig.kit().slots[1].file;
    assert!(file.reverse && !file.overlapping_voices && rig.hits().is_empty());
}

#[test]
fn shift_and_stretch_rebuild_the_engine_once_the_finger_lifts_and_cut_goes_live_with_no_undo_step()
{
    // Shift writes MAIN@1037/@1045 and Stretch STR1 (a re-import, here of a pad with no sample); both only when their
    // step or ratio changes, and neither while the finger is down. `Cut...` (MAIN@1049) goes to the playing kit as REV
    // and Poly do. None makes an undo step.
    let mut rig = drum_rig();
    let steps = rig.app.history.steps();
    strip_page(&mut rig, drums::StripPage::Options);
    rig.take();
    let knob = rig.rect_of(strip(0, Ctl::Shift)).center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, 100.0));
    rig.drag_to(knob - vec2(0.0, 216.0));
    assert_eq!(rig.take(), Vec::<String>::new());
    assert_eq!(rig.kit().slots[0].file.beat_offset, 0.0);
    rig.release(knob - vec2(0.0, 216.0));
    rig.frame(vec![]);
    let offset = rig.kit().slots[0].file.beat_offset;
    assert!(offset > 0.0 && (offset * 128.0).fract() == 0.0, "{offset}");
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.click_id(strip(0, Ctl::Cut));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Cut).with("menu").with((0_usize, 2_usize)));
    assert_eq!(rig.kit().slots[0].cut_group, 2);
    assert_eq!(rig.take(), ["SetSetting 1 -1 1049 2.000"]);
    strip_page(&mut rig, drums::StripPage::Pitch);
    let knob = rig.rect_of(strip(0, Ctl::Stretch)).center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, 100.0));
    rig.drag_to(knob - vec2(0.0, 200.0));
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(knob - vec2(0.0, 200.0));
    rig.frame(vec![]);
    let file = &rig.kit().slots[0].file;
    assert!(
        file.stretch_mode == 1 && file.stretch_ratio > 1.0,
        "{file:?}"
    );
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    assert_eq!(rig.app.history.steps(), steps);
}

#[test]
fn the_pads_replace_the_keyboard_and_play_the_drums_channel() {
    let mut rig = drum_rig();
    // Full velocity, and no glissando: with the pads locked (`lockpads`), sliding onto pad 1 plays nothing, and the
    // release ends pad 0.
    rig.app.settings.lock_pads = true;
    rig.press(pad(0));
    rig.drag_to(pad(1));
    rig.release(pad(1));
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    // A pad with no slot plays nothing.
    rig.click(pad(4));
    assert_eq!(rig.take(), Vec::<String>::new());
    // There is no keyboard under them.
    rig.click(E4);
    assert_eq!(rig.take(), Vec::<String>::new());
    // The playlist keeps the current Drums channel's pads: a pad held across the switch sounds until its release.
    rig.press(pad(2));
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 1 50 127"]);
    rig.release(pad(2));
    assert_eq!(rig.take(), ["NoteOff 1 50"]);
    // Pads that go away (the master made current) let go of the pad held.
    rig.press(pad(2));
    rig.app.selected_channel = 0;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 1 50 127", "NoteOff 1 50"]);
    rig.release(pad(2));
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_pad_dropped_on_another_swaps_their_slots_as_one_undo_step() {
    let mut rig = drum_rig();
    rig.app.settings.lock_pads = false;
    let pads = |rig: &Rig| {
        rig.app.song.channels[1]
            .drum_kit
            .as_ref()
            .unwrap()
            .file
            .pads
    };
    let click = |rig: &mut Rig, at: usize| {
        rig.click(pad(at));
        rig.take()
    };
    // Moved past 1.5 U the kick is picked up and its note ends; dropped on pad 3 it trades places with the clap,
    // with nothing for the audio.
    rig.press(pad(0));
    rig.drag_to(pad(3));
    rig.release(pad(3));
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    assert_eq!(pads(&rig)[..5], [3, 1, 2, 0, 4]);
    // Each pad plays the slot its entry names, by the slot's note (48 + its slot index).
    assert_eq!(click(&mut rig, 0), ["NoteOn 1 51 127", "NoteOff 1 51"]);
    assert_eq!(click(&mut rig, 3), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    // One undo step puts it back, and its redo swaps again.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(pads(&rig)[..5], [0, 1, 2, 3, 4]);
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(pads(&rig)[..5], [3, 1, 2, 0, 4]);
    rig.take();
    // Dropped on pad 5 (unused), the kick moves there and pad 3 plays nothing.
    rig.press(pad(3));
    rig.drag_to(pad(5));
    rig.release(pad(5));
    assert_eq!(pads(&rig)[..6], [3, 1, 2, 5, 4, 0]);
    rig.take();
    assert_eq!(click(&mut rig, 5), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    assert_eq!(click(&mut rig, 3), Vec::<String>::new());
    rig.frame(z(egui::Modifiers::NONE));
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(pads(&rig)[..6], [0, 1, 2, 3, 4, 5]);
    rig.take();
    // Locked pads stay where they are.
    rig.app.settings.lock_pads = true;
    rig.press(pad(0));
    rig.drag_to(pad(5));
    rig.release(pad(5));
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    assert_eq!(pads(&rig)[..6], [0, 1, 2, 3, 4, 5]);
}

#[test]
fn the_keys_take_no_touch_with_the_master_alone_and_key_lock_snaps_a_major_songs_c_sharp() {
    // The master alone: the keys are disabled, so a press is neither played nor drawn.
    let mut rig = Rig::new();
    let lit = |rig: &Rig| rig.key_level(E4) > 0.0;
    rig.click(OUTSIDE);
    rig.take();
    rig.press(E4);
    assert!(!lit(&rig));
    rig.release(E4);
    assert_eq!(rig.take(), Vec::<String>::new());
    // C major: C# (centred on 2 U) plays C on its left half and D on its right; E is in the scale.
    let mut rig = Rig::with_synth();
    rig.app.song.key_scale = crate::song::scales::scale_to_stored(0);
    for (x, key) in [(1.8, 48), (2.2, 50), (5.0, 52)] {
        let at = pos2(x * U, 445.5);
        rig.press(at);
        rig.release(at);
        let on = rig.take().first().cloned().unwrap_or_default();
        assert!(on.starts_with(&format!("NoteOn 1 {key} ")), "{x} U: {on}");
    }
}

impl Rig {
    /// The hexagons the last frame painted in the keyboard band: the pads, if they show.
    fn pad_hexes(&self) -> usize {
        let hex = |s: &&egui::epaint::ClippedShape| matches!(&s.shape, Shape::Path(p) if p.points.len() == 6 && KEYBOARD.contains(p.points[0]));
        self.out.shapes.iter().filter(hex).count()
    }
}

#[test]
fn the_bottom_panel_follows_the_current_channels_first_lane_in_every_view() {
    // The current channel's first lane picks the panel, not the open editor: a drum lane the pads, anything else the
    // keyboard (`drum_editor::pads_channel` has every case).
    let mut rig = drum_rig();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    // An editor follows the current track: the master's makes the master current, whose keys play nothing.
    rig.app.song.channels[0].lanes[0].add_clip(0, 4.0);
    let master = ClipKey {
        channel: 0,
        lane: 0,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(master));
    rig.slide_split();
    rig.take();
    assert_eq!(rig.app.selected_channel, 0);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(0));
    assert_eq!(rig.pad_hexes(), 0);
    rig.press(E4);
    rig.release(E4);
    assert!(rig.take().iter().all(|c| !c.starts_with("NoteOn")));
    // The Drums channel current: its pads, on the playlist too.
    rig.app.selected_channel = 1;
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert!(rig.pad_hexes() > 0);
    rig.press(pad(1));
    rig.release(pad(1));
    assert_eq!(rig.take(), ["NoteOn 1 49 127", "NoteOff 1 49"]);
    // The first lane, not the channel's kind: a MiniSynth whose first lane is a drum lane shows pads, all unused (it
    // has no kit), which play nothing.
    rig.app.song.channels[2].lanes[0].kind = LaneKind::Drums;
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert!(rig.pad_hexes() > 0);
    rig.press(pad(0));
    rig.release(pad(0));
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn the_drum_editor_of_a_channel_without_pads_falls_back_to_the_playlist() {
    let mut rig = Rig::with_synth();
    rig.app.navigate(Nav::Show(View::DrumEditor(1)));
    rig.slide_split();
    assert_eq!(rig.app.nav.view, View::Playlist);
}

#[test]
fn a_new_drums_channels_default_drumset_shows_and_its_pads_play_it() {
    // The add-channel `Drums` opens the drumset picker; its `Default` is the built-in kit.
    let mut rig = Rig::new();
    rig.app.add_channel(ChannelKind::Drums, 0);
    super::rack_wiring::picker_row(&mut rig, 1);
    rig.frame(vec![]);
    assert!(rig.app.nav.overlays.is_empty());
    rig.click(OUTSIDE);
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    rig.take();
    let kit = DrumKit::default_kit();
    // One row per pad, named after it: the rows on screen, then the rest after a drag up to the end.
    let mut rows = std::collections::BTreeMap::new();
    let mut read_rows = |rig: &Rig| {
        for row in 0..=kit.slots.len() {
            let id = egui::Id::new("step_seq").with(("row", row));
            if let Some(header) = rig.ctx.read_response(id) {
                // A row read on screen once: the drag leaves the top one under the bar's labels.
                rows.entry(row)
                    .or_insert_with(|| rig.texts_in(header.rect).swap_remove(0));
            }
        }
    };
    read_rows(&rig);
    let grid = rig.rect_of(steps("grid")).center();
    rig.press(grid);
    // Under the playlist (`ui::split`) the editor shows fewer rows: read them on the way too.
    for dy in 1..=10 {
        rig.drag_to(grid - vec2(0.0, 50.0 * dy as f32));
        read_rows(&rig);
    }
    rig.release(grid - vec2(0.0, 500.0));
    rig.frame(vec![]);
    rig.frame(vec![]);
    read_rows(&rig);
    let pads: Vec<&str> = kit.slots.iter().map(SamplerSlot::pad_name).collect();
    assert_eq!(rows.into_values().collect::<Vec<_>>(), pads);
    // Pad i plays its slot, key 48 + i; the pads past the kit play nothing. The second pad row is below the default
    // 5 U panel: an 8 U panel brings it on screen (the panel takes a few frames to grow).
    rig.app.settings.preferred_kbd_size = 8.0;
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    let pad = |i| pad_in(8.0, i);
    assert!(pad(kit.slots.len()).y < 540.0);
    for i in 0..kit.slots.len() {
        rig.press(pad(i));
        rig.release(pad(i));
        let key = 48 + i;
        assert_eq!(
            rig.take(),
            [format!("NoteOn 1 {key} 127"), format!("NoteOff 1 {key}")]
        );
    }
    rig.click(pad(kit.slots.len()));
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn an_undo_removes_a_new_drums_channel_with_its_empty_pad() {
    let mut rig = Rig::new();
    let mut before = rig.app.song.clone();
    rig.app.add_channel(ChannelKind::Drums, 0);
    // A new Drums channel: one empty pad; the drumset picker, closed, loads nothing.
    let kit = || {
        let mut kit = DrumKit::default();
        kit.insert_slot(0, SamplerSlot::default());
        Some(kit)
    };
    assert_eq!(rig.app.song.channels[1].drum_kit, kit());
    rig.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    rig.frame(vec![]);
    rig.frame(z(egui::Modifiers::NONE));
    // The undo counts as editing time, which is no edit.
    before.file.meta.editing_seconds = rig.app.song.file.meta.editing_seconds;
    assert_eq!(rig.app.song, before);
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(rig.app.song.channels[1].drum_kit, kit());
}

// ---- Fixes ----

fn note(pos: u32, key: u8) -> Event {
    Event::Note {
        pos,
        len_beats: 1.0,
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

#[test]
fn ghost_notes_show_another_channels_notes_while_the_setting_is_on() {
    let mut rig = Rig::with_drums_and_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let bass = rig.app.song.channels[3].lanes[0].add_clip(0, 4.0);
    bass.events.push(note(0, 60));
    rig.edit(2);
    rig.slide_split();
    let hue = rig.app.song.channels[3].hue;
    let ghost = channel_color(hue, &rig.app.theme)
        .to_color32()
        .gamma_multiply(crate::ui::piano_roll::GHOST_ALPHA);
    assert_eq!(rig.rects(ghost).len(), 1);
    rig.app.settings.ghost_notes = false;
    rig.frame(vec![]);
    assert_eq!(rig.rects(ghost).len(), 0);
}

/// A key held on the keyboard lights its row in the roll of its channel (the channel colour α 0.1), until it is let go.
#[test]
fn a_held_keyboard_key_lights_its_row_in_the_roll() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let lit = channel_color(rig.app.song.channels[1].hue, &rig.app.theme)
        .to_color32()
        .gamma_multiply(0.1);
    let grid = rig.rect_of(roll("grid"));
    let rows = |rig: &Rig| -> Vec<Rect> {
        let shown = |r: &&Rect| r.intersects(grid) && r.height() < U;
        rig.rects(lit).iter().filter(shown).copied().collect()
    };
    assert_eq!(rows(&rig), []);
    rig.press(E4);
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.piano_roll.sounding, 1 << 52);
    let held = rows(&rig);
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].x_range(), grid.x_range());
    rig.release(E4);
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rows(&rig), []);
}

#[test]
fn the_rolls_c_labels_follow_the_middle_c_setting() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    // The key column, left of the grid and below the ruler: the roll opens on middle C.
    let keys = Rect::from_min_max(pos2(0.0, EDITOR_TOP + RULER), pos2(1.5 * U, PANEL_TOP));
    assert_eq!(rig.texts_in(keys), ["C5"]);
    rig.app.settings.middle_c_is_c4 = true;
    rig.frame(vec![]);
    assert_eq!(rig.texts_in(keys), ["C4"]);
}

#[test]
fn the_rolls_playhead_is_the_transports_and_its_line_shows_stopped_and_playing() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    rig.rec.0.borrow_mut().tick = 300;
    rig.edit(1);
    rig.slide_split();
    // Stopped: the play box and the line show the transport's tick 300 (a beat, 128 ticks, being 2 U): the line has no
    // play-state condition.
    let left = rig.rect_of(roll("grid")).left();
    let at = left + 300.0 / 128.0 * 2.0 * U;
    assert_eq!(rig.playhead(roll("grid")), (at, true));
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(rig.playhead(roll("grid")), (at, true));
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(rig.playhead(roll("grid")), (at, true));
    // A transport outside the clip: it rests at the clip's start, its line with it.
    rig.rec.0.borrow_mut().tick = 5000;
    rig.frame(vec![]);
    assert_eq!(rig.playhead(roll("grid")), (left, true));
}

/// Every editor's play box fades as the drum editor's (`timeline::knob_shown`): out while playing, so that only the
/// line shows, and back in once stopped.
#[test]
fn while_playing_the_playlists_and_the_rolls_play_boxes_fade_out_and_only_their_lines_show() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 8.0);
    rig.rec.0.borrow_mut().tick = 300;
    rig.frame(vec![]);
    let settle = |rig: &mut Rig| (0..40).for_each(|_| rig.frame(vec![]));
    // The playlist: a beat is 1.2 U from the grid's left edge.
    let x = r3(1.5 * U + 2.0 + 300.0 / 128.0 * 1.2 * U);
    assert_eq!(playlist_playhead(&rig), (Some((x, 255)), true));
    rig.click(PLAY);
    rig.frame(vec![]);
    let (disc, line) = playlist_playhead(&rig);
    assert!(disc.is_some_and(|(_, a)| a < 255) && line, "{disc:?}");
    settle(&mut rig);
    assert_eq!(playlist_playhead(&rig), (None, true));
    rig.click(PLAY);
    settle(&mut rig);
    assert_eq!(playlist_playhead(&rig), (Some((x, 255)), true));
    // The roll: a beat is 2 U.
    rig.edit(1);
    rig.slide_split();
    let grid = roll("grid");
    let x = rig.rect_of(grid).left() + 300.0 / 128.0 * 2.0 * U;
    assert_eq!(rig.editor_playhead(grid), (Some((x, 255)), Some(x)));
    rig.click(PLAY);
    rig.frame(vec![]);
    let (disc, line) = rig.editor_playhead(grid);
    assert!(
        disc.is_some_and(|(_, a)| a < 255) && line == Some(x),
        "{disc:?}"
    );
    settle(&mut rig);
    assert_eq!(rig.editor_playhead(grid), (None, Some(x)));
    rig.click(PLAY);
    settle(&mut rig);
    assert_eq!(rig.editor_playhead(grid), (Some((x, 255)), Some(x)));
}

/// The playlist's playhead: its play box on the ruler as its playhead x ([`r3`]) and alpha, if drawn, and whether its
/// line (`text`, narrower than U) runs down the track area.
fn playlist_playhead(rig: &Rig) -> (Option<(f32, u8)>, bool) {
    let text = rig.app.theme.colors.text.to_color32();
    let tracks = egui::Rangef::new(RULER, PANEL_TOP);
    let line = (rig.rects(text).iter()).any(|r| r.width() < U && r.y_range() == tracks);
    (rig.play_box_in(0.0..RULER), line)
}

#[test]
fn the_playlists_playhead_shows_only_over_the_grid_and_fades_in_there() {
    let mut rig = Rig::with_synth();
    // Room to scroll: the timeline scrolls until the last clip's end reaches the grid's left edge.
    rig.app.song.channels[1].lanes[0].add_clip(0, 64.0);
    // The grid starts t = 2 points right of the 1.5 U headers; a beat is 1.2 U (128 ticks) at the default zoom.
    let (grid_left, beat) = (1.5 * U + 2.0, 1.2 * U);
    let at = |rig: &mut Rig, tick: u64, scroll: f64| {
        rig.rec.0.borrow_mut().tick = tick;
        rig.app.main_screen.scroll_x = scroll;
        rig.frame(vec![]);
        playlist_playhead(rig)
    };
    let settle = |rig: &mut Rig| {
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        playlist_playhead(rig)
    };
    // On the grid, from its left edge to near its right one (1170 points).
    let shown = |x: f32| Some((r3(x), 255));
    assert_eq!(at(&mut rig, 0, 0.0), (shown(grid_left), true));
    assert_eq!(
        at(&mut rig, 9 * 128, 0.0),
        (shown(grid_left + 9.0 * beat), true)
    );
    assert_eq!(
        at(&mut rig, 29 * 128, 0.0),
        (shown(grid_left + 29.0 * beat), true)
    );
    assert_eq!(at(&mut rig, 4 * 128, 4.0), (shown(grid_left), true));
    // Past the right edge, or scrolled off left over the headers: no line and no disc, which tracks the live playhead
    // and is not drawn off the ruler; it fades out meanwhile (`timeline::PlayheadKnob`).
    assert_eq!(at(&mut rig, 30 * 128, 0.0), (None, false));
    assert_eq!(at(&mut rig, 3 * 128, 4.0), (None, false));
    assert_eq!(settle(&mut rig), (None, false));
    // Back on the grid it fades in on the transport's tick, and its line shows at once (no play-state condition).
    let x = grid_left + 9.0 * beat;
    assert_eq!(at(&mut rig, 9 * 128, 0.0), (Some((x, 51)), true));
    assert_eq!(settle(&mut rig), (shown(x), true));
    assert_eq!(at(&mut rig, 30 * 128, 0.0), (None, false));
}

#[test]
fn a_note_drag_is_one_ongoing_edit_until_its_release_rebuilds_once() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    rig.take();
    // A tap at the start of the first step adds a note there.
    let grid = rig.rect_of(roll("grid"));
    let at = pos2(grid.left() + 4.0, grid.center().y);
    rig.click(at);
    let [Event::Note { pos: 0, key, .. }] = rig.notes(1)[..] else {
        panic!("{:?}", rig.notes(1));
    };
    // The new note sounds for 150 ms.
    assert_eq!(
        rig.take(),
        [
            format!("NoteOn 1 {key} 127"),
            "ReplaceSong adopting no-resync".into()
        ]
    );
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), [format!("NoteOff 1 {key}")]);
    // Two steps right and three rows up: every frame of the drag is an ongoing edit, which rebuilds nothing; the
    // dragged key is heard.
    let to = at + vec2(32.0, -48.0);
    rig.press(at);
    for k in 1..=4 {
        rig.drag_to(at + (to - at) * k as f32 / 4.0);
        let log = rig.take();
        assert!(
            !log.contains(&"ReplaceSong".to_owned()),
            "frame {k}: {log:?}"
        );
    }
    rig.release(to);
    assert_eq!(
        rig.take(),
        [
            format!("NoteOff 1 {}", key + 3),
            "ReplaceSong adopting".into()
        ]
    );
    assert!(matches!(
        rig.notes(1)[..],
        [Event::Note { pos: 64, key: k, .. }] if k == key + 3
    ));
}

#[test]
fn a_note_dragged_back_to_its_start_leaves_undo_on_the_step_before() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let grid = rig.rect_of(roll("grid"));
    let at = pos2(grid.left() + 4.0, grid.center().y);
    // Two notes, two steps.
    rig.click(at);
    rig.click(at + vec2(64.0, -48.0));
    assert_eq!(rig.notes(1).len(), 2);
    // The new notes' sounds end.
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.take();
    // The first note two steps right and back again.
    rig.press(at);
    rig.drag_to(at + vec2(32.0, 0.0));
    rig.drag_to(at);
    rig.release(at);
    // The release ends the gesture: a final edit, which rebuilds.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // The first undo takes the second note away.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.notes(1).len(), 1);
}

#[test]
fn a_held_pad_lets_go_of_the_key_it_played() {
    let mut rig = drum_rig();
    rig.press(pad(1));
    assert_eq!(rig.take(), ["NoteOn 1 49 127"]);
    // The pad loses its slot while held: its note-off is still key 49.
    let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
    kit.slots.clear();
    rig.release(pad(1));
    assert_eq!(rig.take(), ["NoteOff 1 49"]);
}

#[test]
fn switching_straight_to_another_drum_channel_releases_the_held_pad() {
    let mut rig = drum_rig();
    rig.app.add_default_drums(0);
    rig.app.song.channels[2].drum_kit = rig.app.song.channels[1].drum_kit.clone();
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.take();
    rig.press(pad(0));
    assert_eq!(rig.take(), ["NoteOn 1 48 127"]);
    // The other Drums channel made current: its pads, and the held one let go.
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    // The press carried over plays nothing on the other kit's pads.
    rig.drag_to(pad(1));
    rig.release(pad(1));
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_new_song_resets_the_rolls_and_the_drum_editor() {
    let mut rig = drum_rig();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.edit(2);
    rig.slide_split();
    assert!(rig.app.slots.piano_roll.rolls.contains_key(&2));
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.frame(vec![]);
    assert!(rig.app.slots.piano_roll.rolls.is_empty());
}

#[test]
fn a_new_song_keeps_the_drum_mixer_folded_out() {
    // The mixer's fold is in memory, kept across closing and reopening; only a new run starts collapsed. The rig's
    // mixer is out; a New song (as the main menu's) replaces the song.
    let mut rig = drum_rig();
    rig.app.song = Song::default();
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.frame(vec![]);
    rig.app.add_default_drums(0);
    rig.edit(1);
    rig.slide_split();
    rig.frame(vec![]);
    assert_eq!(rig.rect_of(steps("grid")).left(), 15.0 * U + T);
    assert!(rig.ctx.read_response(drum_editor("pages")).is_some());
}

#[test]
fn a_new_song_keeps_the_free_grids_drum_mixer_folded_out_too() {
    // Each editor keeps its own fold (the step-sequencer and free-grid editors each keep their own state); only a new
    // run starts collapsed. Out on a notes lane, then a New song: a new channel's notes lane opens with it still out.
    use crate::song::{DrumGrid, LaneKind};
    let notes_lane = |rig: &mut Rig| {
        let drums = &mut rig.app.song.channels[1];
        let lane = drums.add_lane(LaneKind::Drums);
        drums.lanes[lane].grid = DrumGrid::Notes;
        drums.lanes[lane].add_clip(0, 4.0);
        rig.app.song_edited(SongEdit::Final, 0);
        let key = ClipKey {
            channel: 1,
            lane,
            id: 1,
        };
        rig.app.navigate(Nav::EditClip(key));
        rig.slide_split();
        rig.frame(vec![]);
    };
    let grid = |name: &str| egui::Id::new("drum_grid").with(name);
    let mut rig = drum_rig();
    notes_lane(&mut rig);
    // A tap on the current row's header folds it out.
    rig.click_id(egui::Id::new("drum_grid").with(("row", 0_usize)));
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.rect_of(grid("grid")).left(), 15.0 * U + T);
    rig.app.song = Song::default();
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.frame(vec![]);
    rig.app.add_default_drums(0);
    notes_lane(&mut rig);
    assert_eq!(rig.rect_of(grid("grid")).left(), 15.0 * U + T);
    assert!(rig.ctx.read_response(drum_editor("pages")).is_some());
}

#[test]
fn the_playlists_view_goes_into_the_song_but_is_no_unsaved_change() {
    // The view (position, zooms) is stored in the song; a scroll is no edit to ask about.
    let mut rig = Rig::new();
    rig.frame(vec![]);
    assert!(!rig.app.files.modified());
    rig.wheel(pos2(24.0, 200.0), 300.0);
    let zoom_y = rig.app.song.file.playlist_view[3];
    // Rows 2 U / (1 − 0.001·300) tall: zoom y is their reciprocal in U.
    assert!((zoom_y - 0.7 / 2.0).abs() < 1e-6, "{zoom_y}");
    assert!(!rig.app.files.modified());
}

// ---- the note menu's items ----

impl Rig {
    /// Centre of the text `label` the last frame painted.
    fn text_center(&self, label: &str) -> Pos2 {
        (self.out.shapes.iter())
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == label => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {label:?}"))
    }

    /// Opens the selection's `More...` and clicks `item`, then lets the dialog it opens (if any) appear.
    fn note_menu(&mut self, item: &str) {
        self.click(self.text_center("More..."));
        self.frame(vec![]);
        self.click(self.text_center(item));
        self.frame(vec![]);
        self.frame(vec![]);
    }
}

#[test]
fn each_note_menu_action_rebuilds_once_and_is_one_undo_step() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let grid = rig.rect_of(roll("grid"));
    let at = pos2(grid.left() + 4.0, grid.center().y);
    rig.click(at);
    let [Event::Note { key, .. }] = rig.notes(1)[..] else {
        panic!("{:?}", rig.notes(1));
    };
    // A long press (a second held still) selects the note.
    rig.press(at);
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.release(at);
    rig.frame(vec![]);
    rig.take();
    let slide = |rig: &Rig| matches!(rig.notes(1)[..], [Event::Note { slide: true, .. }]);

    rig.note_menu("Toggle Slide");
    assert!(slide(&rig));
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);

    // Transpose: nothing is heard while dragging, one rebuild on the release.
    rig.note_menu("Transpose by...");
    let label = rig.text_center("0");
    rig.press(label);
    rig.drag_to(label - vec2(0.0, U));
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(label - vec2(0.0, U));
    assert!(matches!(rig.notes(1)[..], [Event::Note { key: k, slide: true, .. }] if k == key + 2));
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);

    // Each was its own step.
    rig.frame(z(egui::Modifiers::NONE));
    assert!(matches!(rig.notes(1)[..], [Event::Note { key: k, slide: true, .. }] if k == key));
    rig.frame(z(egui::Modifiers::NONE));
    assert!(!slide(&rig));
}

#[test]
fn humanize_previews_until_apply_which_is_one_undo_step_and_cancel_none() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    let grid = rig.rect_of(roll("grid"));
    let at = pos2(grid.left() + 4.0, grid.center().y);
    rig.click(at);
    rig.press(at);
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.release(at);
    rig.frame(vec![]);
    let (before, steps) = (rig.notes(1), rig.app.history.steps());
    let panel = roll("humanize");
    // The base timing offset dragged to its right end: every start moves later.
    let shift = |rig: &mut Rig| {
        let slider = rig.rect_of(panel.with(("row", 7_usize)));
        rig.press(slider.center());
        rig.drag_to(slider.right_center() + vec2(U, 0.0));
        rig.release(slider.right_center() + vec2(U, 0.0));
        rig.frame(vec![]);
    };
    rig.note_menu("Humanize");
    shift(&mut rig);
    assert_eq!(rig.notes(1), before, "the preview leaves the song alone");
    rig.click_id(panel.with("cancel"));
    rig.frame(vec![]);
    assert_eq!(rig.notes(1), before);
    assert_eq!(rig.app.history.steps(), steps);

    rig.note_menu("Humanize");
    shift(&mut rig);
    rig.click_id(panel.with("apply"));
    rig.frame(vec![]);
    // A quarter beat later, give or take the timing's deviation (3.2 ticks).
    let start = |notes: &[Event]| notes.iter().map(Event::pos).sum::<u32>();
    assert!(
        start(&rig.notes(1)) > start(&before) + 16,
        "{:?}",
        rig.notes(1)
    );
    assert_eq!(rig.app.history.steps(), steps + 1);
}

// ---- a pad tap's gate ----

impl Rig {
    /// A tap on `at` within one frame; a touch's pointer goes with the lift.
    fn tap(&mut self, at: Pos2, touch: bool) {
        let mut events = vec![
            egui::Event::PointerMoved(at),
            button_event(at, true),
            button_event(at, false),
        ];
        events.extend(touch.then_some(egui::Event::PointerGone));
        self.frame(events);
    }

    /// The next frame comes a quarter of a tap's gate after the last one.
    fn next_frame_soon(&mut self) {
        self.time += TAP_GATE / 4.0 - 0.1;
    }

    /// Two frames within the gate of a tap in the last one: egui asks for the frame after an input by itself.
    fn two_frames_in_the_gate(&mut self) {
        for _ in 0..2 {
            self.next_frame_soon();
            self.frame(vec![]);
        }
    }

    /// The last frame asks for another by the end of a tap's gate.
    fn repaints_within_the_gate(&self) -> bool {
        self.out.viewport_output[&egui::ViewportId::ROOT].repaint_delay
            <= std::time::Duration::from_secs_f64(TAP_GATE)
    }
}

#[test]
fn a_pad_tap_within_one_frame_sounds_until_its_gate_ends() {
    let mut rig = drum_rig();
    for touch in [false, true] {
        rig.tap(pad(0), touch);
        let tapped = rig.time;
        assert_eq!(rig.take(), ["NoteOn 1 48 127"]);
        // Frames within the gate keep the note, and ask for the frame that ends it; the first one after it stops
        // the note.
        rig.assert_gate(tapped, "NoteOff 1 48");
    }
    // A tap on a pad with no slot plays nothing and leaves the tapped note to its gate.
    rig.tap(pad(0), false);
    rig.next_frame_soon();
    rig.tap(pad(4), false);
    assert_eq!(rig.take(), ["NoteOn 1 48 127"]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
}

#[test]
fn a_pad_played_within_a_tap_gate_stops_the_tapped_note_first() {
    let mut rig = drum_rig();
    // Pressed and held: it sounds until its release, the gate's end stops nothing more.
    rig.tap(pad(0), false);
    rig.next_frame_soon();
    rig.frame(vec![
        egui::Event::PointerMoved(pad(1)),
        button_event(pad(1), true),
    ]);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 48 127", "NoteOff 1 48", "NoteOn 1 49 127"]
    );
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(pad(1));
    assert_eq!(rig.take(), ["NoteOff 1 49"]);
    // Tapped, another pad or the same one: it has a gate of its own.
    for other in [2, 1] {
        rig.tap(pad(1), false);
        rig.next_frame_soon();
        rig.tap(pad(other), false);
        let key = 48 + other as u8;
        assert_eq!(
            rig.take(),
            [
                "NoteOn 1 49 127".to_owned(),
                "NoteOff 1 49".to_owned(),
                format!("NoteOn 1 {key} 127")
            ]
        );
        rig.next_frame_soon();
        rig.frame(vec![]);
        assert_eq!(rig.take(), Vec::<String>::new());
        rig.frame(vec![]);
        assert_eq!(rig.take(), [format!("NoteOff 1 {key}")]);
    }
}

#[test]
fn a_pad_tap_gate_outlives_its_pads_and_a_song_replace_stops_its_note() {
    let mut rig = drum_rig();
    rig.app.add_default_drums(0);
    rig.app.song.channels[2].drum_kit = rig.app.song.channels[1].drum_kit.clone();
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.take();
    // The pads go within the gate (the master made current): the note still stops at its end.
    rig.tap(pad(0), false);
    rig.app.selected_channel = 0;
    rig.two_frames_in_the_gate();
    assert_eq!(rig.take(), ["NoteOn 1 48 127"]);
    assert!(rig.repaints_within_the_gate());
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    // Straight to another Drums channel's pads: the note stops on its own channel.
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.tap(pad(0), false);
    rig.app.selected_channel = 2;
    rig.next_frame_soon();
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOn 1 48 127"]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    // A song replace within the gate stops the note in the engine it played on.
    rig.tap(pad(0), false);
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.next_frame_soon();
    rig.frame(vec![]);
    assert_eq!(
        rig.take(),
        ["NoteOn 2 48 127", "NoteOff 2 48", "Stop", "ReplaceSong"]
    );
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

// ---- a focus loss stops every pad note at once ----

#[test]
fn a_focus_loss_stops_a_held_or_tapped_pad_at_once() {
    let mut rig = drum_rig();
    let lost = || vec![egui::Event::WindowFocused(false)];
    // Held: egui keeps the button down when the focus goes, and the release may be lost with it.
    rig.press(pad(0));
    rig.frame(lost());
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    // The stale press plays nothing, a new one does.
    rig.frame(vec![egui::Event::WindowFocused(true)]);
    rig.drag_to(pad(1));
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.press(pad(1));
    rig.release(pad(1));
    assert_eq!(rig.take(), ["NoteOn 1 49 127", "NoteOff 1 49"]);
    // Tapped: a hidden web page runs no frame to end its gate, so the focus-loss frame does.
    rig.tap(pad(0), false);
    rig.next_frame_soon();
    rig.frame(lost());
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_song_replace_stops_a_held_pad_or_key_in_the_engine_it_played_on() {
    // A failed build keeps the old engine playing, so its notes stop before the replace.
    let mut rig = drum_rig();
    rig.press(pad(0));
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.frame(vec![]);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 48 127", "NoteOff 1 48", "Stop", "ReplaceSong"]
    );
    rig.release(pad(0));
    assert_eq!(rig.take(), Vec::<String>::new());

    let mut rig = Rig::with_synth();
    rig.press(E4);
    rig.app.test_edit = Some(SongEdit::Replaced);
    rig.frame(vec![]);
    assert_eq!(
        rig.take(),
        ["NoteOn 1 52 108", "NoteOff 1 52", "Stop", "ReplaceSong"]
    );
}

// ---- the step selection's floating buttons ----

#[test]
fn the_step_selections_copy_and_delete_go_through_the_funnel_as_one_undo_step_each() {
    let mut rig = drum_rig();
    rig.click(rig.step(0, 0));
    rig.click(rig.step(1, 2));
    rig.take();
    // A long press (a second held still) on the Kick's hit, then a drag to the Snare's: a box over both.
    let (from, to) = (rig.step(0, 0), rig.step(1, 2));
    rig.press(from);
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.drag_to(to);
    rig.release(to);
    rig.frame(vec![]);
    assert_eq!(rig.take(), Vec::<String>::new(), "selecting edits nothing");

    // Copy: the two hits again three steps on, one rebuild, and a re-sync as the piano roll's Copy at interval.
    rig.click(rig.text_center("Copy"));
    assert_eq!(rig.hits(), [(0, 0), (1, 64), (0, 96), (1, 160)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // The selection is the copies now: Delete takes them away again.
    rig.frame(vec![]);
    rig.click(rig.text_center("Delete"));
    assert_eq!(rig.hits(), [(0, 0), (1, 64)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);

    // Each was its own step.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.hits(), [(0, 0), (1, 64), (0, 96), (1, 160)]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.hits(), [(0, 0), (1, 64)]);
}

#[test]
fn a_clips_edit_opens_that_drum_clip_whose_steps_land_in_it() {
    // The drum editor shown on channel 1's first clip, then the playlist.
    let mut rig = drum_rig();
    // The rig's editor opened on the clip at 0 (`Rig::edit`).
    let lane = &mut rig.app.song.channels[1].lanes[0];
    lane.add_clip(512, 4.0);
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    rig.click_id(egui::Id::new(EDITOR_CLOSE));
    // `Edit` on the second clip (bar 2: a beat is 38.4 points from x 48).
    rig.click(pos2(48.0 + 6.0 * 38.4, header(1).y));
    rig.frame(vec![]);
    rig.click_id(egui::Id::new(("clip_button", 3_usize)));
    assert_eq!(rig.app.nav.view, View::DrumEditor(1));
    rig.slide_split();
    rig.click(rig.step(0, 1));
    let clips = &rig.app.song.channels[1].lanes[0].clips;
    let hits: Vec<usize> = clips.iter().map(|c| c.events.len()).collect();
    assert_eq!(hits, [0, 1]);
}

#[test]
fn the_wheel_scrolls_no_piano_roll_under_the_rack_but_beside_it() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.app.navigate(Nav::Open(Overlay::Rack(1)));
    rig.slide_split();
    // The key column: the roll opens on middle C.
    let keys = Rect::from_min_max(pos2(0.0, EDITOR_TOP + RULER), pos2(1.5 * U, PANEL_TOP));
    assert_eq!(rig.texts_in(keys), ["C5"]);
    rig.wheel(pos2(800.0, 200.0), 500.0);
    assert_eq!(rig.texts_in(keys), ["C5"]);
    rig.wheel(pos2(300.0, 200.0), 500.0);
    assert_ne!(rig.texts_in(keys), ["C5"]);
}

#[test]
fn the_wheel_zooms_no_piano_roll_under_the_main_menu() {
    let mut rig = Rig::with_synth();
    rig.edit(1);
    rig.slide_split();
    // The key column: at the default zoom it shows middle C only; the wheel over it zooms the rows out, and it shows
    // more Cs: their channel-colour bars, as rows that short have no labels. (Above the graph lane's tab, whose icon
    // is in the channel colour too.)
    let keys = Rect::from_min_max(
        pos2(0.0, EDITOR_TOP + RULER),
        pos2(1.5 * U, PANEL_TOP - 1.25 * U),
    );
    let ink =
        crate::theme::channel_color(rig.app.song.channels[1].hue, &rig.app.theme).to_color32();
    let zoom_out = |rig: &mut Rig| {
        rig.wheel(pos2(0.75 * U, 200.0), -800.0);
        let bar = |r: &egui::epaint::RectShape| r.fill == ink && keys.contains(r.rect.center());
        (rig.out.shapes.iter())
            .filter(|s| matches!(&s.shape, Shape::Rect(r) if bar(r)))
            .count()
    };
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    assert_eq!(zoom_out(&mut rig), 1);
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    assert!(zoom_out(&mut rig) > 1);
}

#[test]
fn the_wheel_and_a_pinch_move_no_step_rows_under_the_main_menu() {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    let slot = |n: usize| SamplerSlot {
        name: format!("S{n}"),
        folder: format!("Drum Samples\\S{n}"),
        ..SamplerSlot::default()
    };
    rig.app.song.channels[1].drum_kit = Some(DrumKit {
        slots: (0..20).map(slot).collect(),
        ..DrumKit::default()
    });
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    // The rows' zoom and scroll and the time zoom (a trackpad pinch over the grid zooms the time).
    let views = |rig: &Rig| -> Vec<(f32, f32, f32)> {
        let views = rig.app.slots.drum_editor.step.views.values();
        views.map(|v| (v.rows.pitch, v.rows.scroll, v.k)).collect()
    };
    let at = pos2(800.0, 200.0);
    let pinch = |rig: &mut Rig| rig.two_finger_zoom(at);
    rig.wheel(at, -300.0);
    let before = views(&rig);
    rig.app
        .navigate(Nav::Open(Overlay::MainMenu(MenuTab::Settings)));
    rig.frame(vec![]);
    rig.wheel(at, -300.0);
    pinch(&mut rig);
    assert_eq!(views(&rig), before);
    rig.app.nav.overlays.clear();
    rig.frame(vec![]);
    pinch(&mut rig);
    assert_ne!(views(&rig), before);
}

// ---- the rulers' gestures and the region menu ----

impl Rig {
    /// A long press on a ruler at `from` (held 0.3 s), dragged to `to` and released, then a tap on the `+` under `to`
    /// and on the region menu's `item`.
    fn region(&mut self, from: Pos2, to: Pos2, item: &str) {
        self.press(from);
        self.next_frame_at(self.time + 0.31);
        self.frame(vec![]);
        self.drag_to(to);
        self.release(to);
        self.frame(vec![]);
        self.click(self.text_center("+"));
        self.frame(vec![]);
        self.click(self.text_center(item));
    }

    /// (start, visible ticks) of channel 1's clips.
    fn clip_spans(&self) -> Vec<(u32, f64)> {
        (self.app.song.channels[1].lanes[0].clips.iter())
            .map(|c| (c.start_tick, c.visible_ticks()))
            .collect()
    }
}

#[test]
fn a_playlist_region_edit_rebuilds_once_and_is_one_undo_step_that_keeps_the_loop() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(512, 4.0).events = vec![note(0, 60)];
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    rig.take();
    // Beats 2 to 6 on the ruler (a beat is 38.4 points from x 48): the clip's first two beats go.
    let beat = |b: f32| pos2(48.0 + b * 38.4, 24.0);
    rig.region(beat(2.0), beat(6.0), "Delete space");
    assert_eq!(rig.clip_spans(), [(256, 256.0)]);
    // The region's loop reached the engine; the edit clears it, and the new engine reads none.
    assert_eq!(rig.take(), ["SetLoop 256 768", "ReplaceSong adopting"]);
    assert_eq!(rig.app.song.play_loop, None);
    // A loop set since is no edit, and the undo keeps it.
    rig.app.song.play_loop = Some((0, 128));
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.clip_spans(), [(512, 512.0)]);
    assert_eq!(rig.app.song.play_loop, Some((0, 128)));
}

#[test]
fn a_play_loop_is_no_undo_step_even_when_a_final_edit_follows_it() {
    let mut rig = Rig::with_synth();
    let before = rig.app.song.clone();
    rig.app.song.tempo_bpm = 99.0;
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    rig.app.files.saved();
    // A loop on the playlist's ruler: a long press on beat 2 dragged to beat 6 (a beat is 38.4 points from x 48).
    let beat = |b: f32| pos2(48.0 + b * 38.4, 24.0);
    rig.press(beat(2.0));
    rig.next_frame_at(rig.time + 0.31);
    rig.frame(vec![]);
    rig.drag_to(beat(6.0));
    rig.release(beat(6.0));
    assert_eq!(rig.app.song.play_loop, Some((256, 768)));
    // No song-changed flag either.
    assert!(!rig.app.files.modified());
    // A final edit that changes nothing (a knob dragged back to where it started) records no step for the loop.
    rig.app.test_edit = Some(SongEdit::Final);
    rig.frame(vec![]);
    // Ctrl+Z undoes the tempo, the last edit, keeps the loop, and undoes nothing else.
    rig.frame(z(egui::Modifiers::NONE));
    let mut want = Song {
        play_loop: Some((256, 768)),
        ..before
    };
    // The editing time counts on: it is no edit either.
    want.file.meta.editing_seconds = rig.app.song.file.meta.editing_seconds;
    assert_eq!(rig.app.song, want);
}

#[test]
fn piano_roll_and_drum_editor_region_edits_go_through_the_funnel_as_one_undo_step_each() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![note(0, 60), note(256, 67)];
    rig.app.test_edit = Some(SongEdit::Final);
    rig.edit(1);
    rig.slide_split();
    rig.take();
    // The roll: a beat is 64 points from x 48; `Insert space` on beats 0 to 2 (made from its end, clear of the play
    // box's touch area at beat 0).
    let beat = |b: f32| pos2(48.0 + b * 64.0, EDITOR_TOP + 24.0);
    rig.region(beat(2.0), beat(0.1), "Insert space");
    let starts = |rig: &Rig| rig.notes(1).iter().map(Event::pos).collect::<Vec<_>>();
    assert_eq!(starts(&rig), [256, 512]);
    // The region's loop reached the engine before the edit.
    assert_eq!(rig.take(), ["SetLoop 0 256", "ReplaceSong adopting"]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(starts(&rig), [0, 256]);

    // The drum editor: Kick on step 0 and Snare on step 2; `Trim selection` over steps 2 to 4 (made from its end, clear
    // of the play box's touch area at step 0) keeps the Snare, now on step 0.
    let mut rig = drum_rig();
    rig.click(rig.step(0, 0));
    rig.click(rig.step(1, 2));
    rig.take();
    let step = |s: usize| pos2(rig.step(0, s).x, EDITOR_TOP + 24.0);
    rig.region(step(4), step(2), "Trim selection");
    assert_eq!(rig.hits(), [(1, 0)]);
    assert_eq!(rig.take(), ["SetLoop 64 128", "ReplaceSong adopting"]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.hits(), [(0, 0), (1, 64)]);
}

#[test]
fn a_tap_on_any_ruler_sets_the_shared_playhead_or_stops_a_playing_transport() {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(512, 8.0);
    // The playlist's ruler: beat 3 (a beat is 38.4 points from x 48).
    let ruler = pos2(48.0 + 3.1 * 38.4, 24.0);
    rig.click(ruler);
    assert_eq!(rig.take(), ["Seek 384"]);
    rig.rec.0.borrow_mut().tick = 384;
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 600;
    rig.take();
    rig.click(ruler);
    // Back to the play-start point, as REV's first press.
    assert_eq!(rig.take(), ["Stop", "Seek 384"]);
    assert!(!rig.app.main_screen.playing);
    // The roll on the clip at bar 2: its content tick 256 is song tick 768.
    rig.edit(1);
    rig.slide_split();
    rig.click(pos2(48.0 + 2.0 * 64.0, EDITOR_TOP + 24.0));
    assert_eq!(rig.take(), ["Seek 768"]);
    // Stopped there, the roll's disc and line show it.
    rig.rec.0.borrow_mut().tick = 768;
    rig.frame(vec![]);
    let left = rig.rect_of(roll("grid")).left();
    assert_eq!(rig.playhead(roll("grid")), (left + 2.0 * 2.0 * U, true));

    // The drum editor's ruler (a new Drums channel's lane has no clip yet): step 4 is tick 128.
    let mut rig = drum_rig();
    rig.click(pos2(rig.step(0, 4).x, EDITOR_TOP + 24.0));
    assert_eq!(rig.take(), ["Seek 128"]);
}

#[test]
fn the_playhead_knob_takes_its_own_taps_and_the_ruler_beside_it_keeps_its_gestures() {
    // The handle (1.75 U across, riding the playhead) once swallowed the ruler's gestures near it. The handle is now
    // The playhead knob, with gestures of its own (`app/tests/transport.rs`): the ruler keeps the rest.
    let mut rig = Rig::with_synth();
    rig.rec.0.borrow_mut().tick = 384;
    rig.frame(vec![]);
    // Stopped at beat 3; 0.6 beat to its right is on the box (a beat is 38.4 points from x 48): a tap plays.
    let beat = |b: f32| pos2(48.0 + b * 38.4, 24.0);
    rig.click(beat(3.6));
    assert_eq!(rig.take(), ["Play"]);
    rig.click(PLAY);
    rig.take();
    // Past the box's touch area (2.29 U right of the playhead, to beat 4.9), a tap seeks...
    rig.click(beat(5.2));
    assert_eq!(rig.take(), ["Seek 640"]);
    // ... and a long press starts a range.
    rig.press(beat(5.4));
    rig.next_frame_at(rig.time + 0.31);
    rig.frame(vec![]);
    rig.drag_to(beat(7.0));
    rig.release(beat(7.0));
    assert_eq!(rig.app.song.play_loop, Some((640, 896)));
}

// ---- the graph lanes ----

/// The graph lanes' knobs on the editor's bottom edge while the lanes are hidden: the roll's at 0.75 U, the drum
/// editor's at 1.4 U.
const ROLL_KNOB: Pos2 = pos2(0.75 * U, PANEL_TOP - 4.0);
const DRUM_KNOB: Pos2 = pos2(1.4 * U, PANEL_TOP - 4.0);

/// The note or hit values `prop` of `events`.
fn values_of(events: &[Event], prop: crate::ui::graph_lane::Prop) -> Vec<u16> {
    events.iter().filter_map(|e| prop.get(e)).collect()
}

/// MiniSynth (1) holding `notes` (a final edit) in its piano roll, the graph lane shown, an empty log.
fn roll_lane_rig(notes: Vec<Event>) -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.song.channels[1].lanes[0].add_clip(0, 4.0).events = notes;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    assert!(
        rig.ctx.read_response(roll("graph")).is_none(),
        "hidden first"
    );
    rig.click(ROLL_KNOB);
    rig.frame(vec![]);
    rig.take();
    rig
}

#[test]
fn a_drag_in_the_rolls_graph_lane_is_heard_once_and_undone_in_one_step() {
    use crate::ui::graph_lane::Prop;
    let mut rig = roll_lane_rig(vec![note(0, 60), note(256, 64)]);
    // Halfway up over the first note, then above the lane over the second (a beat is 2 U): nothing is heard until
    // the release.
    let bars = rig.rect_of(roll("graph"));
    rig.press(pos2(bars.left() + 10.0, bars.center().y));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [127, 204]);
    rig.drag_to(pos2(bars.left() + 5.0 * U, 0.0));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [127, 255]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(pos2(bars.left() + 5.0 * U, 0.0));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    // One undo step, and one redo.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [204, 204]);
    rig.frame(z(egui::Modifiers::SHIFT));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [127, 255]);
}

#[test]
fn a_roll_lane_drag_back_to_its_start_closes_its_undo_step() {
    use crate::ui::graph_lane::Prop;
    let mut rig = roll_lane_rig(vec![note(0, 60)]);
    rig.click(rig.text_center("Pan"));
    // From the centre (127) up to 254 and back, then released: the edit is closed and heard.
    let bars = rig.rect_of(roll("graph"));
    let x = bars.left() + 10.0;
    rig.press(pos2(x, bars.center().y));
    rig.drag_to(pos2(x, 0.0));
    assert_eq!(values_of(&rig.notes(1), Prop::Pan), [254]);
    rig.drag_to(pos2(x, bars.center().y));
    rig.release(pos2(x, bars.center().y));
    assert_eq!(values_of(&rig.notes(1), Prop::Pan), [127]);
    // A touch that changed nothing: its own undo re-syncs.
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // So an undo steps back past it, to before the note.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.notes(1), []);
}

#[test]
fn an_undo_mid_drag_in_the_rolls_lane_reverts_it_and_ends_it() {
    use crate::ui::graph_lane::Prop;
    let mut rig = roll_lane_rig(vec![note(0, 60), note(256, 64)]);
    let bars = rig.rect_of(roll("graph"));
    let (a, b) = (bars.left() + 10.0, bars.left() + 5.0 * U);
    rig.press(pos2(a, bars.center().y));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [127, 204]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [204, 204]);
    // The rest of the drag paints nothing, and its release is no edit.
    rig.drag_to(pos2(b, bars.top()));
    rig.release(pos2(b, bars.top()));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [204, 204]);
    assert_eq!(
        rig.take(),
        ["ReplaceSong adopting"],
        "the undo's rebuild only"
    );
    // Nothing to redo, and the next undo is the edit before the drag.
    assert!(!rig.app.history.can_redo());
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.notes(1), []);
}

#[test]
fn a_drag_in_the_drum_editors_graph_lane_is_heard_once_and_undone_in_one_step() {
    use crate::ui::graph_lane::Prop;
    let mut rig = drum_rig();
    assert!(
        rig.ctx.read_response(steps("graph")).is_none(),
        "hidden first"
    );
    rig.click(DRUM_KNOB);
    rig.frame(vec![]);
    let hits = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.clone();
    rig.click(rig.step(0, 0));
    rig.click(rig.step(0, 2));
    rig.take();
    // The selected row (Kick), whose tapped hits are velocity 255: halfway up on step 0, then below the lane on step 2.
    let bars = rig.rect_of(steps("graph"));
    let (a, b) = (rig.step(0, 0).x, rig.step(0, 2).x);
    rig.press(pos2(a, bars.center().y));
    rig.drag_to(pos2(b, bars.bottom() + 10.0));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [127, 1]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(pos2(b, bars.bottom() + 10.0));
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 255]);
    // An undo mid-drag reverts the drag and ends it: the rest of it paints nothing.
    rig.press(pos2(b, bars.center().y));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 127]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 255]);
    rig.drag_to(pos2(a, bars.bottom() + 10.0));
    rig.release(pos2(a, bars.bottom() + 10.0));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 255]);
    // The other items edit the other fields: Repeat at the top is 7.
    rig.click(rig.text_center("Repeat"));
    rig.click(pos2(a, bars.top() + 1.0));
    assert_eq!(values_of(&hits(&rig), Prop::Repeat), [7, 0]);
}

#[test]
fn an_undo_mid_drag_in_the_drum_lane_reverts_it_and_ends_it() {
    // The roll's case, for the drum editor's lane.
    use crate::ui::graph_lane::Prop;
    let mut rig = drum_rig();
    rig.click(DRUM_KNOB);
    rig.frame(vec![]);
    let hits = |rig: &Rig| rig.app.song.channels[1].lanes[0].clips[0].events.clone();
    rig.click(rig.step(0, 0));
    rig.click(rig.step(0, 2));
    rig.take();
    let bars = rig.rect_of(steps("graph"));
    let (a, b) = (rig.step(0, 0).x, rig.step(0, 2).x);
    rig.press(pos2(a, bars.center().y));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [127, 255]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 255]);
    // The rest of the drag paints nothing. Its release finds the hits as at the touch-down (the view cannot tell the
    // undo from its own edits): a touch that changed nothing, whose own undo re-syncs, and no undo step.
    rig.drag_to(pos2(b, bars.bottom()));
    rig.release(pos2(b, bars.bottom()));
    assert_eq!(values_of(&hits(&rig), Prop::Velocity), [255, 255]);
    assert_eq!(rig.take(), ["ReplaceSong adopting", "ReplaceSong adopting"]);
    // Nothing to redo, and the next undo is the edit before the drag: the second hit's.
    assert!(!rig.app.history.can_redo());
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.hits(), [(0, 0)]);
}

/// A graph-lane touch that changes nothing undoes its own snapshot, and that undo re-syncs; the history keeps no step
/// of it. In the roll's lane and the drum editor's.
#[test]
fn a_graph_lane_touch_that_changes_nothing_re_syncs() {
    let mut rig = roll_lane_rig(vec![note(0, 60)]);
    let bars = rig.rect_of(roll("graph"));
    // Past the note's beat: no note to paint.
    let at = pos2(bars.right() - 10.0, bars.center().y);
    rig.press(at);
    rig.release(at);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    // So an undo steps back past it, to before the note.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.notes(1), []);

    let mut rig = drum_rig();
    rig.click(DRUM_KNOB);
    rig.frame(vec![]);
    rig.click(rig.step(0, 0));
    rig.take();
    let bars = rig.rect_of(steps("graph"));
    // Step 5 holds no hit; step 0's hit is at 255 already, the lane's top.
    for at in [
        pos2(rig.step(0, 5).x, bars.center().y),
        pos2(rig.step(0, 0).x, bars.top()),
    ] {
        rig.press(at);
        rig.release(at);
        assert_eq!(rig.take(), ["ReplaceSong adopting"], "{at:?}");
    }
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.hits(), []);
}

/// The no-op touch's re-sync is no edit. After an edit and its undo, it keeps the redo, makes no undo step and leaves
/// The song as modified as it was.
#[test]
fn a_no_op_lane_touch_after_an_undo_keeps_the_redo() {
    use crate::ui::graph_lane::Prop;
    let mut rig = roll_lane_rig(vec![note(0, 60)]);
    let bars = rig.rect_of(roll("graph"));
    let x = bars.left() + 10.0;
    rig.press(pos2(x, bars.top()));
    rig.release(pos2(x, bars.top()));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [255]);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(values_of(&rig.notes(1), Prop::Velocity), [204]);
    rig.take();
    let (steps, modified) = (rig.app.history.steps(), rig.app.files.modified());
    // Past the note's beat: nothing to paint.
    let at = pos2(bars.right() - 10.0, bars.center().y);
    rig.press(at);
    rig.release(at);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert!(rig.app.history.can_redo());
    assert_eq!(rig.app.history.steps(), steps);
    assert_eq!(rig.app.files.modified(), modified);
}

#[test]
fn a_graph_lane_drag_the_drum_editor_hides_mid_way_closes_its_undo_step_in_either_view() {
    // F9 `showMainMixer`, `Select clip` or `Nav::Show(Playlist)` mid-drag, then the release: the step stayed
    // open and the paint merged into the next edit. As a note drag: the hidden editor's paint is one final edit.
    use crate::song::{DrumGrid, LaneKind};
    use crate::ui::graph_lane::Prop;
    for free in [false, true] {
        let mut rig = drum_rig();
        let lane = if free {
            let drums = &mut rig.app.song.channels[1];
            let notes = drums.add_lane(LaneKind::Drums);
            drums.lanes[notes].grid = DrumGrid::Notes;
            drums.lanes[notes].add_clip(0, 4.0).events = vec![Event::Drum {
                pos: 0,
                len_beats: 0.25,
                pad: 0,
                velocity: 255,
                pan: 127,
                value: 32767,
                repeat: 0,
            }];
            rig.app.song_edited(SongEdit::Final, 0);
            let key = ClipKey {
                channel: 1,
                lane: notes,
                id: 1,
            };
            rig.app.navigate(Nav::EditClip(key));
            rig.slide_split();
            rig.frame(vec![]);
            notes
        } else {
            rig.click(rig.step(0, 0));
            0
        };
        let view = egui::Id::new(if free { "drum_grid" } else { "step_seq" });
        rig.click(DRUM_KNOB);
        rig.frame(vec![]);
        rig.take();
        let velocity = |rig: &Rig| {
            values_of(
                &rig.app.song.channels[1].lanes[lane].clips[0].events,
                Prop::Velocity,
            )
        };
        assert_eq!(velocity(&rig), [255]);
        let bars = rig.rect_of(view.with("graph"));
        let at = pos2(bars.left() + 6.0, bars.center().y);
        rig.press(at);
        rig.drag_to(at + vec2(2.0, 0.0));
        assert_eq!(velocity(&rig), [127], "free {free}");
        assert_eq!(rig.take(), Vec::<String>::new());
        let steps = rig.app.history.steps();
        rig.app.navigate(Nav::Show(View::Playlist));
        rig.frame(vec![]);
        let log = rig.take();
        assert!(
            log.len() == 1 && log[0].starts_with("ReplaceSong adopting"),
            "{log:?}"
        );
        assert_eq!(rig.app.history.steps(), steps + 1, "free {free}");
        rig.release(at + vec2(2.0, 0.0));
        rig.frame(vec![]);
        assert_eq!(rig.app.history.steps(), steps + 1, "free {free}");
        assert!(!rig.app.history.can_redo());
        // Its own step: the undo takes the paint back, and only it.
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(velocity(&rig), [255], "free {free}");
        assert_eq!(rig.app.history.steps(), steps, "free {free}");
    }
}

#[test]
fn the_drum_lanes_knob_is_over_the_strips_of_a_full_kit() {
    // The default kit's eight rows reach the editor's bottom edge, where the hidden lane's knob sits over row 4's
    // strip.
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    rig.edit(1);
    rig.slide_split();
    rig.click(DRUM_KNOB);
    rig.frame(vec![]);
    let values = rig.rect_of(steps("graph"));
    assert_eq!(values.bottom(), PANEL_TOP);
    // And on the lane's top it hides it again.
    rig.click(pos2(DRUM_KNOB.x, values.top() - 4.0));
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert!(rig.ctx.read_response(steps("graph")).is_none());
}

// ---- A hidden web page stops every note without a frame ----

#[test]
fn hiding_the_page_stops_every_note_at_once_and_its_return_frame_sends_nothing() {
    // A hidden page runs no frame: `lib.rs` calls `on_hidden` from its `visibilitychange`. eframe queues the focus
    // loss with the hide, so the return frame has it, then the focus; the release was lost, so the button is down.
    let back = || {
        vec![
            egui::Event::WindowFocused(false),
            egui::Event::WindowFocused(true),
        ]
    };
    let mut rig = drum_rig();
    // A held pad: stopped now, and neither held nor drawn held on return.
    rig.press(pad(0));
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    rig.frame(back());
    rig.drag_to(pad(1));
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(rig.app.slots.drum_editor.drums.fingers.is_empty());
    rig.release(pad(1));
    // A tapped pad within its gate: stopped now, not again when the return frame is past the gate.
    rig.tap(pad(0), false);
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    rig.frame(back());
    assert_eq!(rig.take(), Vec::<String>::new());

    // The keyboard's held and tapped keys alike.
    let mut rig = Rig::with_synth();
    rig.press(E4);
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 52"]);
    rig.frame(back());
    rig.drag_to(F4);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(F4);
    rig.tap(E4, false);
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 52"]);
    rig.frame(back());
    assert_eq!(rig.take(), Vec::<String>::new());
    // Nothing sounds: a second stop sends nothing.
    rig.app.on_hidden();
    assert_eq!(rig.take(), Vec::<String>::new());

    // A typed note (`app/typed.rs`): its key's release on return sends nothing.
    let mut rig = Rig::with_synth();
    rig.type_key(egui::Key::Z, true);
    rig.take();
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOff 1 48"]);
    rig.frame(back());
    rig.type_key(egui::Key::Z, false);
    assert_eq!(rig.take(), Vec::<String>::new());

    // The open picker's audition note, held by Enter on its strip.
    use super::instruments_picker::{CH, PLUCK, instrument};
    use super::typed::typed;
    let enter = |pressed| vec![typed(egui::Key::Enter, pressed, egui::Modifiers::NONE)];
    let mut rig = instrument();
    super::rack_wiring::click_row(&mut rig, PLUCK);
    super::rack_wiring::click_row(&mut rig, 0);
    rig.frame(vec![]);
    rig.take();
    rig.frame(enter(true));
    assert_eq!(rig.take(), [format!("NoteOn {CH} 60 127")]);
    rig.app.on_hidden();
    assert_eq!(rig.take(), [format!("NoteOff {CH} 60")]);
    rig.frame(back());
    rig.frame(enter(false));
    assert_eq!(rig.take(), Vec::<String>::new());
}

#[test]
fn a_press_held_over_the_hide_stays_ignored_until_its_release_without_any_focus_event() {
    // A first touch before any release can leave the canvas unfocused (eframe focuses it on `pointerup`), and then
    // eframe queues no focus loss with the hide: the return frame has no focus event, and the button is still down.
    let mut rig = drum_rig();
    rig.press(pad(0));
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    rig.frame(vec![]);
    rig.drag_to(pad(1));
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(rig.app.slots.drum_editor.drums.fingers.is_empty());
    // Its release ends it; a new press plays.
    rig.release(pad(1));
    rig.press(pad(1));
    assert_eq!(rig.take(), ["NoteOn 1 49 127"]);

    // The key lights while held, then fades.
    let mut rig = Rig::with_synth();
    rig.press(E4);
    assert!((rig.key_level(E4) - 0.9).abs() < 0.01);
    rig.app.on_hidden();
    assert_eq!(rig.take(), ["NoteOn 1 52 108", "NoteOff 1 52"]);
    rig.frame(vec![]);
    let fading = rig.key_level(E4);
    assert!(fading < 0.8, "{fading}");
    rig.drag_to(F4);
    assert_eq!(rig.take(), Vec::<String>::new());
    assert!(rig.key_level(E4) < fading);
    assert_eq!(rig.key_level(F4), 0.0);
    rig.release(F4);
    rig.press(E4);
    assert_eq!(rig.take(), ["NoteOn 1 52 108"]);
    assert!(rig.key_level(E4) > 0.9);
}

#[test]
fn a_notes_lanes_clip_opens_the_drum_editor_in_free_grid_mode_and_a_step_lanes_in_step_mode() {
    // The drum editor follows the current lane: kind 3 with flag 0 (DESc@1) opens the step sequencer, kind 3 with
    // flag 1 the free note-grid.
    use crate::song::{DrumGrid, LaneKind};
    let mut rig = drum_rig();
    let drums = &mut rig.app.song.channels[1];
    drums.lanes[0].add_clip(0, 4.0);
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = DrumGrid::Notes;
    drums.lanes[notes].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.take();
    let open = |rig: &mut Rig, lane| {
        let key = ClipKey {
            channel: 1,
            lane,
            id: 1,
        };
        rig.app.navigate(Nav::EditClip(key));
        rig.slide_split();
        assert_eq!(rig.app.nav.view, View::DrumEditor(1));
        let swing = rig.texts_in(Rect::EVERYTHING).contains(&"SWING".to_owned());
        (rig.app.slots.drum_editor.mode(&rig.app.song), swing)
    };
    // The free note-grid (the view) has no SWING; the pads still play.
    assert_eq!(open(&mut rig, notes), (Some(DrumGrid::Notes), false));
    rig.press(pad(0));
    rig.release(pad(0));
    assert_eq!(rig.take(), ["NoteOn 1 48 127", "NoteOff 1 48"]);
    assert_eq!(open(&mut rig, 0), (Some(DrumGrid::Steps), true));
}

#[test]
fn the_free_grid_adds_and_moves_hits_in_one_undo_step_each() {
    // In the free note-grid a tap creates a hit, one undo step; a drag moves it, its release one undo step.
    use crate::song::{DrumGrid, LaneKind};
    let mut rig = drum_rig();
    let drums = &mut rig.app.song.channels[1];
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = DrumGrid::Notes;
    drums.lanes[notes].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let key = ClipKey {
        channel: 1,
        lane: notes,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.slide_split();
    rig.frame(vec![]);
    rig.take();
    let hits = |rig: &Rig| -> Vec<(u8, u32)> {
        let events = &rig.app.song.channels[1].lanes[notes].clips[0].events;
        (events.iter())
            .map(|e| match *e {
                Event::Drum { pad, pos, .. } => (pad, pos),
                _ => panic!("{e:?}"),
            })
            .collect()
    };
    let undo = |rig: &mut Rig| {
        rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
        rig.frame(vec![]);
    };
    // At a new clip's 4 U a beat, a point is a tick; rows are 2 U.
    let grid = rig.rect_of(egui::Id::new("drum_grid").with("grid"));
    let at = |pad: f32, tick: f32| grid.min + vec2(tick, (pad + 0.5) * 2.0 * U);
    rig.click(at(1.0, 47.0));
    rig.frame(vec![]);
    assert_eq!(hits(&rig), [(1, 32)]);
    // The tap's edit is final: the audio is rebuilt at once.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    undo(&mut rig);
    assert_eq!(hits(&rig), []);
    rig.click(at(1.0, 47.0));
    rig.frame(vec![]);
    rig.take();
    // A drag over several frames: nothing is rebuilt until its release, and one step goes back to where it started.
    rig.press(at(1.0, 40.0));
    for dx in [20.0, 45.0, 70.0] {
        rig.drag_to(at(1.0, 40.0) + vec2(dx, 2.0 * U));
    }
    assert_eq!(hits(&rig), [(2, 96)]);
    assert_eq!(rig.take(), Vec::<String>::new());
    rig.release(at(1.0, 40.0) + vec2(70.0, 2.0 * U));
    rig.frame(vec![]);
    assert_eq!(hits(&rig), [(2, 96)]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    undo(&mut rig);
    assert_eq!(hits(&rig), [(1, 32)]);
}

#[test]
fn a_free_grid_drag_the_editor_hides_mid_way_closes_its_undo_step() {
    // The hidden free grid ends a drag whose ongoing edits went out; they become one final edit, one undo step.
    use crate::song::{DrumGrid, LaneKind};
    let mut rig = drum_rig();
    let drums = &mut rig.app.song.channels[1];
    let notes = drums.add_lane(LaneKind::Drums);
    drums.lanes[notes].grid = DrumGrid::Notes;
    drums.lanes[notes].add_clip(0, 4.0).events = vec![Event::Drum {
        pos: 32,
        len_beats: 0.25,
        pad: 1,
        velocity: 255,
        pan: 127,
        value: 32767,
        repeat: 0,
    }];
    rig.app.song_edited(SongEdit::Final, 0);
    let key = ClipKey {
        channel: 1,
        lane: notes,
        id: 1,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.slide_split();
    rig.frame(vec![]);
    rig.take();
    let pos = |rig: &Rig| match rig.app.song.channels[1].lanes[notes].clips[0].events[0] {
        Event::Drum { pos, .. } => pos,
        ref e => panic!("{e:?}"),
    };
    let grid = rig.rect_of(egui::Id::new("drum_grid").with("grid"));
    let from = grid.min + vec2(40.0, 1.5 * 2.0 * U);
    rig.press(from);
    rig.drag_to(from + vec2(70.0, 0.0));
    assert_eq!(pos(&rig), 96);
    assert_eq!(rig.take(), Vec::<String>::new());
    let steps = rig.app.history.steps();
    rig.app.navigate(Nav::Show(View::Playlist));
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["ReplaceSong adopting"]);
    assert_eq!(rig.app.history.steps(), steps + 1);
    rig.release(from + vec2(70.0, 0.0));
    rig.frame(vec![]);
    assert_eq!(rig.app.history.steps(), steps + 1);
    assert_eq!(pos(&rig), 96);
}

// ---- The sample picker and the row menu's drumset items ----

mod samples {
    use std::path::Path;

    use super::*;
    use crate::app::tests::rack_wiring::{dialog_save, picker_row};
    use crate::flm::chunk::{ContentPath, ContentRoot};
    use crate::project_files::PresetKind;
    use crate::song::{DrumGrid, LaneKind};
    use crate::ui::rack::PresetTarget;
    use crate::ui::views::sample_picker::TITLE;

    /// The two drum editors: the step sequencer's widgets, and the free note-grid's.
    const STEPS: &str = "step_seq";
    const GRID: &str = "drum_grid";

    /// [`drum_rig`] with two samples the session kept (the web's files): `My Samples/Kick 01.wav` and
    /// `My Recordings/Take.wav`. With `grid`, the drum editor shows a notes lane of channel 1: the free note-grid.
    fn sample_rig(grid: bool) -> Rig {
        let mut rig = drum_rig();
        for path in ["My Samples/Kick 01.wav", "My Recordings/Take.wav"] {
            (rig.app.files).keep_user_file(Path::new(path), b"RIFF".to_vec());
        }
        if grid {
            let drums = &mut rig.app.song.channels[1];
            let notes = drums.add_lane(LaneKind::Drums);
            drums.lanes[notes].grid = DrumGrid::Notes;
            drums.lanes[notes].add_clip(0, 4.0);
            rig.app.song_edited(SongEdit::Final, 0);
            let key = ClipKey {
                channel: 1,
                lane: notes,
                id: 1,
            };
            rig.app.navigate(Nav::EditClip(key));
            rig.slide_split();
            rig.frame(vec![]);
        }
        rig.take();
        rig
    }

    fn view(grid: bool) -> &'static str {
        if grid { GRID } else { STEPS }
    }

    /// Long-presses row `row`'s header in the editor `view` and picks the row-menu item at (column, row).
    fn row_item(rig: &mut Rig, view: &str, row: usize, item: (usize, usize)) {
        rig.frame(vec![]);
        rig.frame(vec![]);
        let header = rig.rect_of(egui::Id::new(view).with(("row", row)));
        // Its part in the editor, under the playlist (`ui::split`) and above the transport; at its right end, clear of
        // the graph-lane knob's touch box on the editor's bottom edge.
        let shown = Rect::from_x_y_ranges(header.x_range(), EDITOR_TOP..=PANEL_TOP);
        rig.secondary_click(pos2(
            header.right() - 0.25 * U,
            header.intersect(shown).center().y,
        ));
        rig.frame(vec![]);
        rig.click_id(steps("row menu").with(item));
    }

    /// Scrolls the rows to their end, where the `+` is, and clicks it.
    fn add_drum(rig: &mut Rig, view: &str) {
        // The wheel over the grid scrolls the rows (over the headers it would zoom them).
        let grid = rig.rect_of(egui::Id::new(view).with("grid"));
        rig.wheel(grid.center(), -1000.0);
        rig.click_id(egui::Id::new(view).with("add"));
    }

    /// In the open sample picker: `My Files`, then its folder `folder` (0 `My Recordings`, 1 `My Samples`), then that
    /// folder's first file; then it closes the picker, which stays open after a pick.
    fn pick(rig: &mut Rig, folder: usize) {
        picker_row(rig, 0);
        picker_row(rig, folder);
        picker_row(rig, 0);
        rig.frame(vec![]);
        super::rack_wiring::close_picker(rig);
    }

    /// Presses and releases `key`.
    fn press(rig: &mut Rig, key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        rig.frame(vec![event(true), event(false)]);
    }

    fn undo(rig: &mut Rig) {
        rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
        rig.frame(vec![]);
    }

    fn user(relative: &str) -> Option<ContentPath> {
        Some(ContentPath::Shorthand {
            root: ContentRoot::User,
            relative: relative.into(),
        })
    }

    /// The kit's pads, as (pad name, sample name).
    fn names(kit: &DrumKit) -> Vec<(String, String)> {
        (kit.slots.iter())
            .map(|s| (s.pad_name().to_owned(), s.name.clone()))
            .collect()
    }

    /// Row `row`'s header lines in the editor `view`: pad name, then sample name.
    fn header_lines(rig: &mut Rig, view: &str, row: usize) -> Vec<String> {
        rig.frame(vec![]);
        rig.texts_in(rig.rect_of(egui::Id::new(view).with(("row", row))))
    }

    #[test]
    fn load_sample_gives_the_rows_pad_the_file_at_once_with_no_undo_step_in_both_drum_modes() {
        // `Load Sample` opens the sample picker for this row. The pick sets the slot's sample path, folder and sample
        // name: the header's lines.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let before = rig.kit().clone();
            row_item(&mut rig, view(grid), 1, (0, 0));
            assert_eq!(
                rig.overlays(),
                [Overlay::SamplePicker {
                    channel: 1,
                    pad: Some(1)
                }]
            );
            let steps = rig.app.history.steps();
            picker_row(&mut rig, 0);
            rig.frame(vec![]);
            // `My Files`, under the picker's title.
            let crumb = format!("     {TITLE} | My Files");
            assert!(rig.color_of(&crumb).is_some(), "the breadcrumb");
            assert!(
                rig.color_of("My Samples").is_some() && rig.color_of("My Recordings").is_some()
            );
            picker_row(&mut rig, 1);
            rig.frame(vec![]);
            assert!(rig.color_of("Kick 01").is_some(), "listed by name");
            picker_row(&mut rig, 0);
            rig.frame(vec![]);
            // It stays open; closing keeps the pick.
            let open = Overlay::SamplePicker {
                channel: 1,
                pad: Some(1),
            };
            assert_eq!(rig.overlays(), [open]);
            assert_eq!(
                rig.take(),
                ["ReplaceSong adopting no-resync"],
                "grid {grid}"
            );
            super::rack_wiring::close_picker(&mut rig);
            assert_eq!(rig.overlays(), []);
            let slot = &rig.kit().slots[1];
            assert_eq!(slot.file.path, user("My Samples/Kick 01.wav"));
            assert_eq!((slot.file.id, slot.folder.as_str()), (2, "My Samples"));
            assert_eq!(
                header_lines(&mut rig, view(grid), 1),
                ["My Samples", "Kick 01"]
            );
            // No undo step.
            assert_eq!(rig.app.history.steps(), steps);
            assert_ne!(rig.kit(), &before);
        }
    }

    #[test]
    fn the_strip_icon_button_makes_its_row_current_and_opens_the_pad_sample_picker_in_both_drum_modes()
     {
        // Icon 21 is the row menu's `Load Sample`: a pick loads into this pad at once, keeping its STR1 stretch and
        // pitch, and the picker stays open; no undo step.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let editor = &mut rig.app.slots.drum_editor;
            (editor.grid.fold.open, editor.grid.fold.c) = (true, 1.0);
            editor.drums.page = drums::StripPage::Pitch;
            let slot = &mut rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[2];
            (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 1.5);
            slot.file.pitch_semitones = 3.0;
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig.take();
            let steps = rig.app.history.steps();
            rig.click_id(strip(2, Ctl::Sample));
            let open = Overlay::SamplePicker {
                channel: 1,
                pad: Some(2),
            };
            assert_eq!(rig.overlays(), std::slice::from_ref(&open), "grid {grid}");
            let editor = &rig.app.slots.drum_editor;
            let current = if grid {
                editor.grid.current
            } else {
                editor.step.selected
            };
            assert_eq!(current, 2, "grid {grid}");
            picker_row(&mut rig, 0);
            picker_row(&mut rig, 1);
            picker_row(&mut rig, 0);
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), [open], "it stays open");
            let slot = &rig.kit().slots[2];
            assert_eq!(slot.file.path, user("My Samples/Kick 01.wav"));
            let file = &slot.file;
            assert_eq!(
                (file.stretch_mode, file.stretch_ratio, file.pitch_semitones),
                (1, 1.5, 3.0)
            );
            assert_eq!(rig.app.history.steps(), steps, "grid {grid}");
        }
    }

    #[test]
    fn a_pick_tries_a_missing_sample_again_and_the_next_engine_has_it() {
        // A sample missing at a build is not read again on each rebuild, but a new pick reads it again.
        let mut rig = drum_rig();
        let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[1].file.path = user("My Samples/Kick 01.wav");
        rig.app.song_edited(SongEdit::Final, 0);
        assert_eq!(rig.app.audio.samples.slot(&rig.kit().slots[1]), None);
        // The file arrives (a project-made FLAC under a `.wav` name); a rebuild alone does not read it again.
        let flac = include_bytes!("../../../crates/audio/tests/fixtures/sine_flac.wav");
        let path = Path::new("My Samples/Kick 01.wav");
        rig.app.files.keep_user_file(path, flac.to_vec());
        rig.app.song_edited(SongEdit::Final, 0);
        assert_eq!(rig.app.audio.samples.slot(&rig.kit().slots[1]), None);
        rig.take();
        // Re-picking it for its pad: the engine built for the pick has it. The picker opens at the pad's sample, in
        // `My Samples`.
        row_item(&mut rig, STEPS, 1, (0, 0));
        picker_row(&mut rig, 0);
        rig.frame(vec![]);
        super::rack_wiring::close_picker(&mut rig);
        // The sample strip plays the pick on the preview player, then the pick's engine.
        assert_eq!(
            rig.take(),
            [
                "Preview 4410 frames from 0",
                "ReplaceSong adopting no-resync"
            ]
        );
        let sample = rig.app.audio.samples.slot(&rig.kit().slots[1]);
        assert_eq!(sample.map(|s| s.frames()), Some(4410));
        assert_eq!(rig.app.audio.samples.missing().count(), 0);
    }

    #[test]
    fn a_song_load_starts_a_new_sample_cache() {
        // A load drops what the old song decoded (the web's memory) and so reads the files missing before again.
        let mut rig = drum_rig();
        let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
        kit.slots[1].file.path = user("My Samples/Kick 01.wav");
        kit.slots[2].folder = "Built-in/Snare".into();
        rig.app.song_edited(SongEdit::Final, 0);
        let song = rig.app.song.clone();
        let samples = |rig: &Rig, pad: usize| rig.app.audio.samples.slot(&rig.kit().slots[pad]);
        assert_eq!(samples(&rig, 1), None);
        let snare = std::sync::Arc::downgrade(&samples(&rig, 2).expect("the built-in snare"));
        // The file arrives (a FLAC-in-`.wav` fixture), then the song is opened again.
        let flac = include_bytes!("../../../crates/audio/tests/fixtures/sine_flac.wav");
        let path = Path::new("My Samples/Kick 01.wav");
        rig.app.files.keep_user_file(path, flac.to_vec());
        rig.app.song = song;
        rig.app.song_edited(SongEdit::Replaced, 0);
        assert_eq!(samples(&rig, 1).map(|s| s.frames()), Some(4410));
        assert!(
            snare.upgrade().is_none(),
            "the old song's samples were kept"
        );
        let kick = std::sync::Arc::downgrade(&samples(&rig, 1).unwrap());
        // New: nothing of the old song is left.
        rig.app.song = crate::song::Song::default();
        rig.app.song_edited(SongEdit::Replaced, 0);
        assert!(kick.upgrade().is_none());
        assert_eq!(rig.app.audio.samples.missing().count(), 0);
    }

    #[test]
    fn the_add_drum_button_adds_the_picked_sample_on_a_new_pad_and_selects_it_in_both_drum_modes() {
        // The `+` add-drum button. A pick adds a slot on the next free pad, with no custom name, and makes its row the
        // current one: one undo step.
        // Closing the picker adds nothing.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let before = rig.kit().clone();
            add_drum(&mut rig, view(grid));
            assert_eq!(
                rig.overlays(),
                [Overlay::SamplePicker {
                    channel: 1,
                    pad: None
                }]
            );
            // Escape: nothing added, no edit.
            picker_row(&mut rig, 0);
            press(&mut rig, egui::Key::Escape);
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), []);
            assert_eq!(rig.take(), Vec::<String>::new());
            assert_eq!(rig.kit(), &before);
            // A pick from My Recordings: pad 4, after Kick, Snare, Hat and Clap.
            let steps = rig.app.history.steps();
            add_drum(&mut rig, view(grid));
            pick(&mut rig, 0);
            assert_eq!(
                rig.take(),
                ["ReplaceSong adopting no-resync"],
                "grid {grid}"
            );
            let kit = rig.kit();
            assert_eq!(kit.slots.len(), 5);
            assert_eq!(kit.slots[4].file.id, 5);
            assert_eq!(kit.slots[4].file.path, user("My Recordings/Take.wav"));
            let editor = &rig.app.slots.drum_editor;
            let current = if grid {
                editor.grid.current
            } else {
                editor.step.selected
            };
            assert_eq!(current, 4);
            assert_eq!(
                header_lines(&mut rig, view(grid), 4),
                ["My Recordings", "Take"]
            );
            // The hex pad shows the folder (shrunk to fit its 1.5 r text box) over the file name; the slot's folder is
            // The file's real one.
            let pads = rig.texts_in(KEYBOARD);
            assert!(pads.contains(&"My R...ings".to_owned()), "{pads:?}");
            assert_eq!(pads.iter().filter(|t| *t == "Take").count(), 1);
            assert_eq!(rig.kit().slots[4].folder, "My Recordings");
            assert_eq!(rig.kit().slots[4].file.custom_pad_name, "");
            // No undo step.
            assert_eq!(rig.app.history.steps(), steps, "grid {grid}");
        }
    }

    #[test]
    fn set_name_shows_the_custom_name_alone_and_an_empty_name_removes_it_in_both_drum_modes() {
        // `Set Name` writes MAIN@1051 and never MAIN@12; the row header shows it alone, the hex pad never. The prompt
        // opens with it; OK on an empty field removes it.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let before = rig.kit().clone();
            row_item(&mut rig, view(grid), 1, (0, 6));
            rig.frame(vec![]);
            rig.frame(vec![egui::Event::Text("Mine".into())]);
            press(&mut rig, egui::Key::Enter);
            rig.frame(vec![]);
            assert_eq!(
                rig.take(),
                ["ReplaceSong adopting no-resync"],
                "grid {grid}"
            );
            let slot = &rig.kit().slots[1];
            assert_eq!(slot.file.custom_pad_name, "Mine");
            assert_eq!(slot.name, before.slots[1].name);
            assert_eq!(header_lines(&mut rig, view(grid), 1), ["Mine"]);
            let pads = rig.texts_in(KEYBOARD);
            assert!(pads.contains(&"Snare".to_owned()) && pads.contains(&"Snare 01".to_owned()));
            assert!(!pads.contains(&"Mine".to_owned()));
            // The prompt starts from the custom name: the field shows it, as the header does.
            row_item(&mut rig, view(grid), 1, (0, 6));
            rig.frame(vec![]);
            let all = rig.texts_in(egui::Rect::EVERYTHING);
            assert_eq!(
                all.iter().filter(|t| *t == "Mine").count(),
                2,
                "grid {grid}"
            );
            for _ in 0.."Mine".len() {
                press(&mut rig, egui::Key::Backspace);
            }
            press(&mut rig, egui::Key::Enter);
            rig.frame(vec![]);
            assert_eq!(rig.kit(), &before);
            assert_eq!(header_lines(&mut rig, view(grid), 1), ["Snare", "Snare 01"]);
        }
    }

    #[test]
    fn the_rows_save_drumset_opens_the_save_drumset_dialog_in_both_drum_modes() {
        // The row's `Save Drumset` opens the save drumset dialog, saving into My Drumsets. Not a song edit.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let song = rig.app.song.clone();
            row_item(&mut rig, view(grid), 2, (0, 2));
            assert_eq!(
                rig.overlays(),
                [Overlay::PresetDialog {
                    name: "Drums".into(),
                    kind: PresetKind::Drumset,
                }]
            );
            dialog_save(&mut rig);
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), []);
            let path = Path::new("My Drumsets/Drums.tmpl");
            let file = crate::flm::read_drumset(&rig.app.files.read_user_file(path).unwrap());
            assert_eq!(names(&file.unwrap()), names(rig.kit()));
            assert_eq!(rig.app.song, song);
            assert_eq!(rig.take(), Vec::<String>::new());
        }
    }

    #[test]
    fn the_rows_load_drumset_opens_the_drumset_picker_and_loads_the_kit_in_both_drum_modes() {
        // The row's `Load Drumset` opens the drumset picker, whose pick replaces the kit in one undo step.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let kit = rig.kit().clone();
            let bytes = crate::flm::write_drumset(&kit).unwrap();
            (rig.app.files).keep_user_file(Path::new("My Drumsets/Mine.tmpl"), bytes);
            // Another kit: two pads.
            rig.app.song.channels[1]
                .drum_kit
                .as_mut()
                .unwrap()
                .slots
                .truncate(2);
            rig.app.song_edited(SongEdit::Final, 0);
            rig.frame(vec![]);
            rig.take();
            let before = rig.kit().clone();
            row_item(&mut rig, view(grid), 1, (0, 1));
            let picker = Overlay::PresetPicker {
                channel: 1,
                target: PresetTarget::Drumset,
            };
            assert_eq!(rig.overlays(), [picker]);
            picker_row(&mut rig, 0);
            picker_row(&mut rig, 0);
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), []);
            assert_eq!(
                rig.take(),
                ["ReplaceSong adopting no-resync"],
                "grid {grid}"
            );
            assert_eq!(names(rig.kit()), names(&kit));
            // The kit's last id is not undone, so no pad id comes back.
            let last = rig.kit().file.last_slot_id;
            undo(&mut rig);
            let mut want = before.clone();
            want.file.last_slot_id = last;
            assert_eq!(rig.kit(), &want);
        }
    }

    #[test]
    fn an_undo_under_the_open_picker_that_changes_the_kit_closes_it_in_both_drum_modes() {
        // Clone Snare (Kick, Snare, Snare*, Hat, Clap), `Load Sample` on row 3 (Hat), then an undo under the picker:
        // row 3 is Clap now. The picker closes rather than load into the wrong pad.
        for grid in [false, true] {
            let mut rig = sample_rig(grid);
            let before = rig.kit().clone();
            row_item(&mut rig, view(grid), 1, (0, 3));
            assert_eq!(rig.kit().slots.len(), 5);
            row_item(&mut rig, view(grid), 3, (0, 0));
            picker_row(&mut rig, 0);
            let last = rig.kit().file.last_slot_id;
            rig.undo_under_modal();
            let mut want = before.clone();
            want.file.last_slot_id = last;
            assert_eq!(rig.kit(), &want, "the clone's id stays taken");
            assert_eq!(rig.overlays(), [], "grid {grid}");
            // Nothing is left to pick into Clap, now on row 3.
            rig.frame(vec![]);
            assert_eq!(rig.overlays(), []);
            assert!(rig.kit().slots.iter().all(|s| s.file.path.is_none()));
        }
    }

    #[test]
    fn the_open_picker_follows_its_channel_when_an_undo_moves_it() {
        // Two Drums channels with the same (default) kit. The first is deleted, the picker opens on the second (now
        // channel 1), and an undo brings the first back in front of it: the pick goes to the second, now channel 2.
        let mut rig = Rig::new();
        rig.app.add_default_drums(0);
        rig.app.add_default_drums(0);
        (rig.app.files).keep_user_file(Path::new("My Samples/Kick 01.wav"), b"RIFF".to_vec());
        rig.click(OUTSIDE);
        let second = rig.app.song.channels[2].id;
        rig.app.song.channels.remove(1);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.edit(1);
        rig.slide_split();
        row_item(&mut rig, STEPS, 0, (0, 0));
        assert_eq!(
            rig.overlays(),
            [Overlay::SamplePicker {
                channel: 1,
                pad: Some(0)
            }]
        );
        picker_row(&mut rig, 0);
        rig.undo_under_modal();
        assert_eq!(rig.app.song.channels.len(), 3);
        assert_eq!(rig.app.song.channels[2].id, second);
        assert_eq!(
            rig.overlays(),
            [Overlay::SamplePicker {
                channel: 2,
                pad: Some(0)
            }]
        );
        picker_row(&mut rig, 1);
        picker_row(&mut rig, 0);
        rig.frame(vec![]);
        let path = |c: usize| {
            let kit = rig.app.song.channels[c].drum_kit.as_ref().unwrap();
            kit.slots[0].file.path.clone()
        };
        assert_eq!((path(1), path(2)), (None, user("My Samples/Kick 01.wav")));
    }

    /// Nothing else puts audio into the web's session, whose pickers would list empty folders. Where the session keeps
    /// the files (the web), the sample picker's `Import` asks for an audio file (the web's `<input type="file">`) and
    /// keeps it in `My Samples`, a taken name as `name (1)`; the open picker lists it at once, and a pick loads it. The
    /// desktop has no `Import`: its pickers list its folders.
    #[test]
    fn import_in_the_sample_picker_keeps_a_picked_audio_file_in_my_samples() {
        use crate::project_files::{ProjectFiles, SAMPLE_EXTS};
        let mut rig = drum_rig();
        // The web names a picked file by its name alone.
        let mem = MemFiles::with_file("Kick 01.wav", b"RIFF picked");
        rig.app.files = ProjectFiles::with_provider(Box::new(mem.clone()));
        let kept = Path::new("My Samples/Kick 01.wav");
        rig.app.files.keep_user_file(kept, b"RIFF kept".to_vec());
        row_item(&mut rig, STEPS, 1, (0, 0));
        picker_row(&mut rig, 0);
        picker_row(&mut rig, 1);
        rig.frame(vec![]);
        assert!(rig.color_of("Kick 01").is_some() && rig.color_of("Kick 01 (1)").is_none());
        rig.click_id(egui::Id::new("preset_picker").with("import"));
        assert!(rig.color_of("Successfully imported").is_some(), "the flash");
        rig.frame(vec![]);
        assert_eq!(mem.0.borrow().accepts, [&SAMPLE_EXTS[..]]);
        let imported = Path::new("My Samples/Kick 01 (1).wav");
        assert_eq!(
            rig.app.files.read_user_file(imported).unwrap(),
            b"RIFF picked"
        );
        assert_eq!(rig.app.files.read_user_file(kept).unwrap(), b"RIFF kept");
        // The picker stays open on its folder, which lists the copy.
        assert_eq!(
            rig.overlays(),
            [Overlay::SamplePicker {
                channel: 1,
                pad: Some(1)
            }]
        );
        rig.frame(vec![]);
        assert!(rig.color_of("Kick 01 (1)").is_some() && rig.color_of("Kick 01").is_some());
        // A-Z by name: `Kick 01`, then `Kick 01 (1)`.
        picker_row(&mut rig, 1);
        rig.frame(vec![]);
        super::rack_wiring::close_picker(&mut rig);
        assert_eq!(rig.overlays(), []);
        assert_eq!(
            rig.kit().slots[1].file.path,
            user("My Samples/Kick 01 (1).wav")
        );
        // The desktop's picker has no `Import`.
        rig.app.files =
            ProjectFiles::on_disk(&std::env::temp_dir().join("honeydew-u09-no-such-folder"));
        // It opens at the pad's sample's folder, which this user folder lacks.
        row_item(&mut rig, STEPS, 1, (0, 0));
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(rig.color_of("Search").is_some() && rig.color_of("Import").is_none());
    }
}

// ---- The automation and audio editors' wiring ----

mod editors {
    use super::*;
    use crate::song::AutomationTarget;
    use crate::ui::preview;
    use crate::ui::track_menu::TrackOp;

    /// The centre of the one-bar clip at beat 0 on track row `row` (a beat is 38.4 points from x 48).
    fn clip(row: usize) -> Pos2 {
        pos2(48.0 + 2.0 * 38.4, header(row).y)
    }

    /// What the transport's Preview would play: the clip the shown editor edits.
    fn current(rig: &Rig) -> Option<ClipKey> {
        preview::current(rig.app.nav.view, &rig.app.slots, &rig.app.song)
    }

    fn shown(rig: &Rig) -> (View, Option<ClipKey>) {
        (rig.app.nav.view, current(rig))
    }

    /// The master and an Audio Clip channel (1), a one-bar clip at bar 1 on each one's main track, in the undo history;
    /// the audio started and an empty log.
    fn two_clips() -> Rig {
        let mut rig = Rig::new();
        rig.app.add_channel(ChannelKind::AudioClip, 0);
        for channel in [0, 1] {
            rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
        }
        rig.app.song_edited(SongEdit::Final, 0);
        rig.click(OUTSIDE);
        rig.take();
        rig
    }

    fn first(channel: usize, lane: usize) -> ClipKey {
        ClipKey {
            channel,
            lane,
            id: 1,
        }
    }

    #[test]
    fn a_clips_edit_opens_the_automation_or_audio_editor_with_undo_and_preview() {
        for (row, view) in [(0, View::AutomationEditor(0)), (1, View::AudioEditor(1))] {
            let mut rig = two_clips();
            rig.click(clip(row));
            rig.frame(vec![]);
            rig.click_id(egui::Id::new(("clip_button", 3_usize)));
            rig.frame(vec![]);
            assert_eq!(shown(&rig), (view, Some(first(row, 0))));
            assert!(rig.painted("SOLO CLIP"), "{view:?}");
            rig.click_text_button("SOLO CLIP");
            assert_eq!(rig.app.preview, Some(first(row, 0)), "{view:?}");
            rig.click_text_button("SOLO CLIP");
            assert_eq!(rig.app.preview, None);
        }
    }

    #[test]
    fn edit_on_an_audio_channel_opens_its_main_tracks_current_clip_else_its_first() {
        let mut rig = Rig::new();
        rig.app.add_channel(ChannelKind::AudioRecord, 0);
        let edit = |rig: &mut Rig| {
            rig.app.navigate(Nav::Edit(1));
            rig.slide_split();
            (rig.app.nav.view, current(rig).map(|k| k.id))
        };
        // No clip to edit: back to the playlist.
        assert_eq!(edit(&mut rig), (View::Playlist, None));
        let lane = &mut rig.app.song.channels[1].lanes[0];
        for start in [0, 512] {
            lane.add_clip(start, 4.0);
        }
        assert_eq!(edit(&mut rig), (View::AudioEditor(1), Some(1)));
        // The track's current clip (DESc@24) when it has one: the one the editor follows.
        assert_eq!(rig.app.song.channels[1].lanes[0].file.current_clip_id, 1);
        rig.app.song.channels[1].lanes[0].file.current_clip_id = 2;
        assert_eq!(edit(&mut rig), (View::AudioEditor(1), Some(2)));
        // A held clip deleted while the editor was away: the track's current clip again, else its first.
        rig.app.navigate(Nav::EditClip(first(1, 0)));
        rig.slide_split();
        for gone in [1, 2] {
            rig.app.navigate(Nav::Show(View::Playlist));
            rig.frame(vec![]);
            let lane = &mut rig.app.song.channels[1].lanes[0];
            if gone == 2 {
                lane.add_clip(1024, 4.0);
            }
            lane.clips.retain(|c| c.id != gone);
            let main = lane.clips.first().map(|c| c.id);
            assert_eq!(edit(&mut rig), (View::AudioEditor(1), main), "{gone}");
        }
    }

    #[test]
    fn an_editor_whose_clip_is_gone_goes_back_to_the_playlist() {
        let mut rig = two_clips();
        for (channel, view) in [(0, View::AutomationEditor(0)), (1, View::AudioEditor(1))] {
            rig.app.navigate(Nav::EditClip(first(channel, 0)));
            rig.slide_split();
            assert_eq!(rig.app.nav.view, view);
            rig.app.song.channels[channel].lanes[0].clips.clear();
            rig.app.song_edited(SongEdit::Final, 0);
            rig.frame(vec![]);
            assert_eq!(shown(&rig), (View::Playlist, None), "{view:?}");
            // The undo brings the clip back, and its Edit the editor.
            rig.frame(z(egui::Modifiers::NONE));
            rig.app.navigate(Nav::EditClip(first(channel, 0)));
            rig.slide_split();
            assert_eq!(shown(&rig), (view, Some(first(channel, 0))));
        }
    }

    #[test]
    fn the_automation_editor_follows_its_track_when_tracks_or_channels_move() {
        let mut rig = Rig::with_drums_and_synth();
        let level = AutomationTarget {
            instance_id: 0,
            param: 0,
        };
        let synth = &mut rig.app.song.channels[2];
        synth.add_automation_lane(level).add_clip(0, 4.0);
        rig.app.song_edited(SongEdit::Final, 0);
        rig.app.navigate(Nav::EditClip(first(2, 1)));
        rig.slide_split();
        assert_eq!(shown(&rig), (View::AutomationEditor(2), Some(first(2, 1))));
        // A track added before it (automation tracks come last), then the channel above deleted.
        rig.app.track_op(2, 0, TrackOp::AddTrack, 0);
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::AutomationEditor(2), Some(first(2, 2))));
        rig.app.delete_channel(1, 0);
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::AutomationEditor(1), Some(first(1, 2))));
        // The undo of each.
        rig.frame(z(egui::Modifiers::NONE));
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::AutomationEditor(2), Some(first(2, 2))));
        rig.frame(z(egui::Modifiers::NONE));
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::AutomationEditor(2), Some(first(2, 1))));
    }

    #[test]
    fn an_audio_editor_whose_track_goes_shows_the_playlist_not_the_main_tracks_clip() {
        let mut rig = two_clips();
        rig.app.track_op(1, 0, TrackOp::AddTrack, 0);
        // A clip on the new track, kept out of the history: the undo goes straight back to before the track.
        rig.app.song.channels[1].lanes[1].add_clip(0, 4.0);
        rig.app.navigate(Nav::EditClip(first(1, 1)));
        rig.slide_split();
        assert_eq!(shown(&rig), (View::AudioEditor(1), Some(first(1, 1))));
        rig.frame(z(egui::Modifiers::NONE));
        assert_eq!(rig.app.song.channels[1].lanes.len(), 1);
        rig.frame(vec![]);
        assert_eq!(shown(&rig), (View::Playlist, None));
        assert!(rig.app.slots.audio_editor.clips.is_empty());
    }

    #[test]
    fn a_double_tap_on_a_clip_opens_the_editor_of_its_lane_kind() {
        // A tap selects and makes the clip current, a tap on it released < 0.3 s after that opens what its `Edit`
        // opens.
        let mut rig = Rig::with_drums_and_synth();
        rig.app.add_channel(ChannelKind::AudioClip, 0);
        for channel in 0..4 {
            rig.app.song.channels[channel].lanes[0].add_clip(0, 4.0);
        }
        rig.app.song_edited(SongEdit::Final, 0);
        rig.click(OUTSIDE);
        let views = [
            View::AutomationEditor(0),
            View::DrumEditor(1),
            View::PianoRoll(2),
            View::AudioEditor(3),
        ];
        for (row, view) in views.into_iter().enumerate() {
            // Released 0.2 s apart (the rig's frames are 0.1 s).
            let at = clip(row);
            rig.frame(vec![egui::Event::PointerMoved(at)]);
            rig.button(at, true);
            rig.button(at, false);
            assert_eq!(rig.app.nav.view, View::Playlist, "{view:?}");
            rig.button(at, true);
            rig.button(at, false);
            rig.frame(vec![]);
            assert_eq!(shown(&rig), (view, Some(first(row, 0))));
            rig.app.navigate(Nav::Show(View::Playlist));
            rig.frame(vec![]);
            // A tap on empty space clears the selection (its floating row could cover the next clip).
            rig.click(pos2(1100.0, header(3).y));
        }
    }
}

// ---- The playlist's mini mixer ----

#[test]
fn a_mini_mixer_edit_reaches_the_audio_as_set_param_and_saves_no_undo_step() {
    // The expanded track headers' controls are the strip's params.
    let mut rig = Rig::with_synth();
    let steps = rig.app.history.steps();
    // A tap on the current track's header (channel 1, row 1) expands the column; it settles at c = 1.
    let header = header(1);
    rig.click(header);
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.take();
    // Its mute switch, 1.5 U … 3 U along the row.
    rig.click(pos2(2.25 * U, header.y));
    assert_eq!(rig.app.song.channels[1].params.0[2], 1.0);
    assert_eq!(rig.take(), ["SetParam 1 0 2 1.000"]);
    // A channel param: the song-changed flag, and no undo step.
    rig.frame(vec![]);
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
}

#[test]
fn the_drum_strips_meters_show_each_slots_peak_from_the_engine() {
    use honeydew_audio::device::{ModuleKey, ModuleMeter};
    // The mixer is out (`drum_rig`): the rows' meters are horizontal bars along the bottom of each row's page.
    let mut rig = drum_rig();
    let ink = channel_color(rig.app.song.channels[1].hue, &rig.app.theme).to_color32();
    let bars = |rig: &Rig| {
        let rows = rig.rect_of(steps("grid")).y_range();
        let left = 3.0 * U;
        (rig.rects(ink).into_iter())
            .filter(|r| r.left() == left && r.height() <= 4.0 && rows.contains(r.top()))
            .count()
    };
    assert_eq!(bars(&rig), 0, "at rest");
    // Slot 2 (Snare, pad 1) peaks at 0 dB on the left, −30 dB on the right.
    let meter = |instance_id, peak_left: f32, peak_right: f32| ModuleMeter {
        key: ModuleKey {
            channel: 1,
            instance_id,
        },
        value: honeydew_audio::MeterSnapshot {
            peak_left,
            peak_right,
            ..Default::default()
        },
        analysis: Vec::new(),
    };
    rig.rec.0.borrow_mut().meters = vec![meter(-2, 1.0, 0.0316)];
    rig.frame(vec![]);
    assert_eq!(bars(&rig), 2, "the snare's L and R");
    // A live feed asks for the next frame within 50 ms.
    assert!(
        rig.out
            .viewport_output
            .values()
            .any(|v| v.repaint_delay <= std::time::Duration::from_millis(50))
    );
}

#[test]
fn the_step_views_swing_popup_marks_its_frames_and_holds_undo_under_it() {
    // The step view's own backdrop (`step_seq::backdrop`), under the swing popup.
    let mut rig = drum_rig();
    let kit = rig.kit().clone();
    rig.click_id(steps("swing button"));
    rig.frame(vec![]);
    assert!(crate::ui::modal_was_open(&rig.ctx), "the popup is open");
    rig.frame(key_tap(egui::Key::Z, egui::Modifiers::COMMAND));
    assert_eq!(rig.kit(), &kit, "held under the popup");
    rig.frame(escape());
    rig.frame(vec![]);
    assert!(!crate::ui::modal_was_open(&rig.ctx), "closed");
}

#[test]
fn a_keys_area_0_tall_hides_the_pads_and_a_press_there_plays_nothing() {
    // A 4.75 U panel with the strip open: K = 4.75 − 1.75 − 3 = 0, which hides the key content. The pads would be drawn
    // below the panel otherwise.
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    // Opened on a panel with keys, the strip keeps its size; then the panel is set.
    rig.app.navigate(Nav::Open(Overlay::KeyboardSettings));
    rig.frame(vec![]);
    rig.app.settings.preferred_kbd_size = 4.75;
    // Pads: 6-point paths under the bar (the keyboard-settings icon's keys are 6-point paths too).
    let under_bar = 540.0 - (4.75 - 1.75) * U;
    let hexes = |rig: &Rig| {
        (rig.out.shapes.iter())
            .filter(|s| match &s.shape {
                Shape::Path(p) => p.points.len() == 6 && p.points.iter().all(|q| q.y > under_bar),
                _ => false,
            })
            .count()
    };
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.app.main_screen.panel, Some(4.75 * U));
    assert_eq!(hexes(&rig), 0);
    rig.take();
    rig.click(pos2(pad(0).x, 539.5));
    assert_eq!(rig.take(), Vec::<String>::new());
    // A hair taller (K > 0): they are laid out again, below the screen's bottom.
    rig.app.settings.preferred_kbd_size = 4.8;
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    assert!(hexes(&rig) >= drums::PAD_COUNT, "{}", hexes(&rig));
}

#[test]
fn rec_armed_gives_the_step_knob_its_record_icon_and_a_tap_on_the_knob_plays() {
    // The app tells the step view REC's arm (the knob's icon 5) and starts playback on its tap.
    let mut rig = drum_rig();
    // On the editor's ruler, under the playlist and its own box (`ui::split`).
    let ruler = EDITOR_TOP..EDITOR_TOP + RULER;
    let play_box = |rig: &Rig| {
        crate::ui::timeline::testing::play_boxes(&rig.out.shapes)
            .find(|b| ruler.contains(&b.rect.center().y))
            .map(|b| b.rect)
    };
    // The REC icon: a `recordColor` disc on the box.
    let rec = crate::theme::RECORD_COLOR.to_color32();
    let armed = |rig: &Rig, on: Rect| {
        (rig.out.shapes.iter())
            .any(|s| matches!(&s.shape, Shape::Circle(c) if c.fill == rec && on.contains(c.center)))
    };
    let knob = play_box(&rig).expect("the stopped box");
    assert!(!armed(&rig, knob));
    rig.app.main_screen.armed = true;
    rig.frame(vec![]);
    assert!(
        armed(&rig, play_box(&rig).expect("the box")),
        "the record icon"
    );
    assert!(!rig.app.main_screen.playing);
    rig.click(knob.center());
    assert!(rig.app.main_screen.playing);
}

/// The whole keyboard panel is `controlFill` (bar, keys and pads alike) under a 1 px `background` line at its top; over
/// The pads, it shows in the 0.35 U gap above them.
#[test]
fn the_keyboard_panel_is_control_fill_under_a_1_px_background_line_behind_the_piano_and_the_pads() {
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
    let regions = Regions::sized(screen, crate::ui::keyboard_settings::PANEL_H * U, 1, &m);
    let panel = Rect::from_min_max(regions.transport.min, regions.keyboard.max);
    let on_top = |rig: &Rig, at: Pos2| {
        (rig.out.shapes.iter().rev()).find_map(|s| match &s.shape {
            Shape::Rect(r) if r.rect.contains(at) => Some(r.fill),
            _ => None,
        })
    };
    for (rig, what) in [(Rig::with_synth(), "piano"), (drum_rig(), "pads")] {
        let colors = &rig.app.theme.colors;
        let (control, bg) = (
            colors.control_fill.to_color32(),
            colors.background.to_color32(),
        );
        assert!(rig.rects(control).contains(&panel), "{what}");
        let px = 1.0 / rig.ctx.pixels_per_point();
        for x in [1.0, 300.0, 1100.0] {
            assert_eq!(
                on_top(&rig, pos2(x, panel.top() + 0.25 * px)),
                Some(bg),
                "{what} {x}"
            );
        }
        if what == "pads" {
            // The gap above the pads: `controlFill` on top, every pad below it (the top row's apexes reach 0.06 r
            // above the pads' area).
            let gap = pos2(300.0, regions.keyboard.top() + 0.1 * U);
            assert_eq!(on_top(&rig, gap), Some(control));
            let apex = (rig.out.shapes.iter())
                .filter_map(|s| match &s.shape {
                    // Under the bar: its keyboard-settings icon has 6-point keys too.
                    Shape::Path(p)
                        if p.points.len() == 6
                            && p.points.iter().all(|q| q.y > regions.transport.bottom()) =>
                    {
                        p.points.iter().map(|q| q.y).reduce(f32::min)
                    }
                    _ => None,
                })
                .reduce(f32::min);
            assert!(apex.is_some_and(|y| y > gap.y), "{apex:?}");
        }
    }
}

/// The channel's key range spans its instrument modules' `ADD1` ranges; a key is in range when it passes the
/// scale test and that range. Two instruments at 52…53 and 60…62 make 52 … 62.
#[test]
fn the_keys_outside_the_span_of_the_channels_instrument_ranges_are_disabled() {
    let mut rig = Rig::with_synth();
    let rack = &mut rig.app.song.channels[1].rack;
    let mut second = rack.modules[0].clone();
    (rack.modules[0].file.key_low, rack.modules[0].file.key_high) = (52, 53);
    (second.file.key_low, second.file.key_high) = (60, 62);
    rack.modules.push(second);
    rig.frame(vec![]);
    let on_top = |at: Pos2| {
        (rig.out.shapes.iter().rev()).find_map(|s| match &s.shape {
            Shape::Rect(r) if r.rect.contains(at) => Some(r.fill),
            _ => None,
        })
    };
    let colors = &rig.app.theme.colors;
    let (white, black_off) = (
        colors.white_key.to_color32(),
        colors.black_key_disabled.to_color32(),
    );
    // E4 (52) and D5 (62) are in range, low on their keys; D#4 (51) and D#5 (63), high on theirs, are not.
    assert_eq!(on_top(E4), Some(white));
    assert_eq!(on_top(pos2(17.0 * U, LOW)), Some(white));
    let high = KEYS_TOP + 0.3 * (540.0 - KEYS_TOP);
    assert_eq!(on_top(pos2(4.0 * U, high)), Some(black_off));
    assert_eq!(on_top(pos2(18.0 * U, high)), Some(black_off));
}

// ---- The drum graph lane's spacer ----

/// The default kit's drum editor (its slots play built-in one-shots, which code makes) on a lane of `grid` mode, with
/// the graph lane shown and the mixer folded in.
fn spacer_rig(grid: crate::song::DrumGrid) -> Rig {
    let mut rig = Rig::new();
    rig.app.add_default_drums(0);
    rig.app.song.channels[1].lanes[0].grid = grid;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    rig.click(DRUM_KNOB);
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.drum_editor.mode(&rig.app.song), Some(grid));
    rig
}

/// The id of the drum editor's view showing: the free note-grid on a notes lane, else the step sequencer.
fn drum_view(rig: &Rig) -> egui::Id {
    let notes = rig.app.slots.drum_editor.mode(&rig.app.song) == Some(crate::song::DrumGrid::Notes);
    egui::Id::new(if notes { "drum_grid" } else { "step_seq" })
}

/// Taps row `row`'s header in the drum view showing, on its top left (a header cut off by the lane shows its top; the
/// lane knob's 2 U box, x 0.4 U..2.4 U, takes the taps in its middle), and lets a fold finish.
fn tap_row(rig: &mut Rig, row: usize) {
    let header = rig.rect_of(drum_view(rig).with(("row", row)));
    rig.click(pos2(header.left() + 6.0, header.top() + 4.0));
    for _ in 0..3 {
        rig.frame(vec![]);
    }
}

/// The 1 px channel-colour columns the last frame showed in the graph lane's spacer, the header column past the 3 U
/// list (to 15 U, the expanded mixer's room), as (pixel column, top, bottom) in order: the quads of a mesh, which egui
/// draws as they are (a 1 px wide `Shape::Rect` becomes a line segment, which at 1 px tall shrinks to nothing). The
/// strips of the rows under the lane are there too, clipped away.
fn spacer_columns(rig: &Rig) -> Vec<(i32, i32, i32)> {
    let ink = channel_color(rig.app.song.channels[1].hue, &rig.app.theme).to_color32();
    let lane = rig.rect_of(drum_view(rig).with("graph"));
    let spacer = Rect::from_x_y_ranges(3.0 * U..=15.0 * U, lane.y_range());
    let quads = |s: &egui::epaint::ClippedShape| {
        let quads = crate::ui::audio_editor::mesh_quads([&s.shape]).into_iter();
        let clip = s.clip_rect;
        quads.map(move |q| (clip, q))
    };
    let mut columns: Vec<_> = (rig.out.shapes.iter())
        .flat_map(quads)
        .filter(|(clip, (r, color))| {
            *color == ink && spacer.contains_rect(*r) && clip.contains_rect(*r)
        })
        .map(|(_, (r, _))| r)
        .inspect(|r| assert_eq!(r.width(), 1.0, "{r:?}"))
        .map(|r| (r.left() as i32, r.top() as i32, r.bottom() as i32))
        .collect();
    columns.sort_unstable();
    columns
}

/// The pixel columns of `columns`.
fn column_xs(columns: &[(i32, i32, i32)]) -> Vec<i32> {
    let xs: std::collections::BTreeSet<_> = columns.iter().map(|c| c.0).collect();
    xs.into_iter().collect()
}

#[test]
fn the_expanded_drum_mixers_graph_lane_spacer_shows_the_current_rows_sample_in_the_channel_colour()
{
    // With the drum mixer expanded, a spacer fills the rest of the header width and shows the current row's sample
    // waveform in the channel colour.
    let mut rig = spacer_rig(crate::song::DrumGrid::Steps);
    assert_eq!(spacer_columns(&rig), [], "collapsed: no spacer");
    tap_row(&mut rig, 0);
    let kick = spacer_columns(&rig);
    // The whole sample across the spacer's 12 U, a column a pixel.
    assert_eq!(column_xs(&kick), (3 * 32..15 * 32).collect::<Vec<_>>());
    // Another current row: its own sample.
    tap_row(&mut rig, 1);
    assert_eq!(rig.app.slots.drum_editor.step.selected, 1);
    let snare = spacer_columns(&rig);
    assert!(!snare.is_empty() && snare != kick, "the Snare's waveform");
    // A slot without a sample leaves it empty.
    rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[1].folder = "Nowhere".into();
    rig.frame(vec![]);
    assert_eq!(spacer_columns(&rig), []);
}

#[test]
fn the_free_note_grids_spacer_shows_its_own_current_rows_sample() {
    // The free note-grid keeps its own current row.
    let mut rig = spacer_rig(crate::song::DrumGrid::Notes);
    assert_eq!(spacer_columns(&rig), [], "collapsed: no spacer");
    tap_row(&mut rig, 0);
    let kick = spacer_columns(&rig);
    assert_eq!(column_xs(&kick), (3 * 32..15 * 32).collect::<Vec<_>>());
    tap_row(&mut rig, 1);
    assert_eq!(rig.app.slots.drum_editor.grid.current, 1);
    let snare = spacer_columns(&rig);
    assert!(!snare.is_empty() && snare != kick, "the Snare's waveform");
}

#[test]
fn a_folding_mixers_spacer_narrows_with_it() {
    // The spacer is the mixer's room, c·12 U, while it folds: the sample fits it.
    let mut rig = spacer_rig(crate::song::DrumGrid::Steps);
    tap_row(&mut rig, 0);
    // Half out: a frame at the same time steps the fold nothing (`Fold::step`).
    rig.app.slots.drum_editor.step.fold.c = 0.5;
    rig.time -= 0.1;
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.drum_editor.step.mixer_w, 6.0 * U);
    let columns = spacer_columns(&rig);
    assert_eq!(column_xs(&columns), (3 * 32..9 * 32).collect::<Vec<_>>());
}

#[test]
fn a_reversed_slots_spacer_waveform_is_mirrored() {
    // Reversed samples are mirrored.
    let mut rig = spacer_rig(crate::song::DrumGrid::Steps);
    tap_row(&mut rig, 0);
    let forward = spacer_columns(&rig);
    rig.app.song.channels[1].drum_kit.as_mut().unwrap().slots[0]
        .file
        .reverse = true;
    rig.frame(vec![]);
    let mut mirrored: Vec<_> = (spacer_columns(&rig).into_iter())
        .map(|(x, top, bottom)| (3 * 32 + 15 * 32 - 1 - x, top, bottom))
        .collect();
    mirrored.sort_unstable();
    assert!(!forward.is_empty());
    assert_eq!(mirrored, forward);
}

// ---- The drum strips and the song-changed flag ----

/// Whether asking for `what` asks about unsaved changes first. Run it last: `Replace::New` with nothing to ask makes
/// The files' song a new one.
fn asks(rig: &mut Rig, what: Replace) -> bool {
    rig.app.files.replace(what);
    let tag = crate::project_files::DISCARD_TAG;
    matches!(rig.app.files.take_overlay(), Some(Overlay::Message { tag: t, .. }) if t == tag)
}

/// Picks `AUX` menu row `row` of strip `slot` on the `Options` page.
fn aux(rig: &mut Rig, slot: usize, row: usize) {
    rig.click_id(strip(slot, Ctl::Aux));
    rig.frame(vec![]);
    rig.click_id(strip(slot, Ctl::Aux).with("menu").with((0_usize, row)));
    rig.frame(vec![]);
}

#[test]
fn rev_poly_cut_shift_and_an_aux_pick_alone_leave_open_and_new_unasked() {
    let mut rig = drum_rig();
    let at = rig.app.song.channels.len();
    let send = rig.app.song.insert_channel(at, ChannelKind::EffectTrack);
    let send = rig.app.song.channels[send.unwrap()].id;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.app.files.saved();
    strip_page(&mut rig, drums::StripPage::Pitch);
    rig.click_id(strip(1, Ctl::Rev));
    strip_page(&mut rig, drums::StripPage::Options);
    rig.click_id(strip(1, Ctl::Poly));
    rig.click_id(strip(0, Ctl::Cut));
    rig.frame(vec![]);
    rig.click_id(strip(0, Ctl::Cut).with("menu").with((0_usize, 2_usize)));
    let knob = rig.rect_of(strip(0, Ctl::Shift)).center();
    rig.press(knob);
    rig.drag_to(knob - vec2(0.0, 216.0));
    rig.release(knob - vec2(0.0, 216.0));
    rig.frame(vec![]);
    // The send channel, after `Default`.
    aux(&mut rig, 1, 1);
    let (a, b) = (&rig.kit().slots[0], &rig.kit().slots[1]);
    assert!(b.file.reverse && !b.file.overlapping_voices && b.file.send_target == send);
    assert!(a.cut_group == 2 && a.file.beat_offset > 0.0, "{a:?}");
    assert!(!rig.app.files.modified());
    assert!(!asks(&mut rig, Replace::Pick));
    assert!(!asks(&mut rig, Replace::New));
}

#[test]
fn a_mute_toggled_back_is_still_a_change() {
    // A bound strip control sets the flag on every value change, with no undo step.
    let mut rig = drum_rig();
    rig.app.files.saved();
    let steps = rig.app.history.steps();
    let muted = |rig: &Rig| rig.kit().slots[0].params[3];
    let before = muted(&rig);
    rig.click_id(strip(0, Ctl::Mute));
    assert_ne!(muted(&rig), before);
    rig.click_id(strip(0, Ctl::Mute));
    rig.frame(vec![]);
    assert_eq!(muted(&rig), before);
    assert_eq!(rig.app.history.steps(), steps);
    assert!(rig.app.files.modified());
    assert!(asks(&mut rig, Replace::New));
}

/// `Reset` from strip `row`'s Stretch knob menu.
fn reset_stretch(rig: &mut Rig, row: usize) {
    rig.secondary_click(rig.rect_of(strip(row, Ctl::Stretch)).center());
    rig.frame(vec![]);
    rig.click(rig.text_center("Reset"));
    rig.frame(vec![]);
}

#[test]
fn stretch_is_a_change_only_when_its_ratio_changes() {
    let mut rig = drum_rig();
    rig.app.files.saved();
    strip_page(&mut rig, drums::StripPage::Pitch);
    // `Reset` at ratio 1: the same ratio.
    reset_stretch(&mut rig, 0);
    assert_eq!(rig.kit().slots[0].file.stretch_ratio, 1.0);
    assert!(!rig.app.files.modified());
    // A new ratio sets it, and going back to 1 leaves it set.
    stretch(&mut rig, 0, 200.0);
    rig.frame(vec![]);
    assert!(rig.kit().slots[0].file.stretch_ratio > 1.0);
    assert!(rig.app.files.modified());
    reset_stretch(&mut rig, 0);
    assert_eq!(rig.kit().slots[0].file.stretch_ratio, 1.0);
    assert!(rig.app.files.modified());
}

#[test]
fn create_fx_channel_is_a_change() {
    let mut rig = drum_rig();
    rig.app.files.saved();
    strip_page(&mut rig, drums::StripPage::Options);
    let channels = rig.app.song.channels.len();
    // `Default`, then `Create FX Channel`: there is no send channel yet.
    aux(&mut rig, 0, 1);
    assert_eq!(rig.app.song.channels.len(), channels + 1);
    assert!(rig.app.files.modified());
    assert!(asks(&mut rig, Replace::Pick));
}

#[test]
fn clear_and_fill_are_changes() {
    // Their step (the `Quiet` edit) is an undo step saved, which sets the flag.
    let mut rig = drum_rig();
    for item in [(1, 2), (1, 3)] {
        rig.app.files.saved();
        rig.row_menu(1, item);
        assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
        assert!(rig.app.files.modified(), "{item:?}");
    }
}
