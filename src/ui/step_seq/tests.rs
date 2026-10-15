use eframe::egui::{self, Color32, Context, Modifiers, PointerButton, RawInput};

use super::edit::*;
use super::*;
use crate::metrics::DEFAULT_UI_SCALE_INDEX;
use crate::song::{ChannelKind, Clip, Event, Lane, Song, TimeSig};
use crate::theme::theme_by_id;
use crate::ui::clips::place_row;

/// The default `TDIV`: 4 steps a beat, 4 beats a bar.
const SIG: TimeSig = TimeSig {
    subdivision: 4,
    beats_per_bar: 4,
};
/// Ticks per step at [`SIG`].
const STEP_TICKS: u32 = 32;

/// A fake kit: (pad name, sample name, MIDI note) per pad.
struct Kit(Vec<(&'static str, &'static str, Option<u8>)>);

impl DrumPads for Kit {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn names(&self, pad: usize) -> (&str, &str) {
        (self.0[pad].0, self.0[pad].1)
    }

    fn midi_note(&self, pad: usize) -> Option<u8> {
        self.0[pad].2
    }

    /// A pad with no sample-name line has a custom name: its pad name (`drums::row_lines`).
    fn custom_name(&self, pad: usize) -> &str {
        match self.0[pad] {
            (name, "", _) => name,
            _ => "",
        }
    }
}

fn kit() -> Kit {
    Kit(vec![
        ("Kick", "808 Kick", None),
        ("Snare", "808 Snare", Some(38)),
        ("Hat", "808 Hat", None),
        ("Clap", "808 Clap", None),
    ])
}

/// A fresh Drums channel: one drum lane, no clips.
fn drums() -> Channel {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    song.channels.remove(1)
}

fn clip(beats: f64) -> Clip {
    Lane::new(LaneKind::Drums).add_clip(0, beats).clone()
}

/// (pad, step, ticks into the step) of each hit, in event order.
fn cells(clip: &Clip) -> Vec<(u8, u32, u32)> {
    clip.events
        .iter()
        .map(|e| match *e {
            Event::Drum { pad, pos, .. } => (pad, pos / STEP_TICKS, pos % STEP_TICKS),
            _ => panic!("not a drum hit: {e:?}"),
        })
        .collect()
}

fn on(pads_steps: &[(u8, u32)]) -> Vec<(u8, u32, u32)> {
    pads_steps.iter().map(|&(p, s)| (p, s, 0)).collect()
}

fn drum(pad: u8, pos: u32) -> Event {
    Event::Drum {
        pos,
        len_beats: 0.25,
        pad,
        velocity: 100,
        pan: 10,
        value: 7,
        repeat: 0,
    }
}

// Pure edits.

/// A tap's toggle with the default remembered values, no swing.
fn tap(clip: &mut Clip, pad: u8, step: usize) {
    toggle(clip, pad, step, 255, 127, (SIG, 0.0));
}

#[test]
fn toggling_a_step_adds_a_hit_with_the_given_values_and_toggling_again_removes_it() {
    let mut c = clip(4.0);
    assert_eq!(toggle(&mut c, 1, 3, 90, 30, (SIG, 0.0)), Some((1, 3)));
    assert_eq!(
        c.events,
        [Event::Drum {
            pos: 96,
            len_beats: 0.25,
            pad: 1,
            velocity: 90,
            pan: 30,
            value: 0x7FFF,
            repeat: 0,
        }]
    );
    tap(&mut c, 2, 3);
    tap(&mut c, 0, 1);
    // Sorted by position; a new hit goes after the ones already at its position.
    assert_eq!(cells(&c), on(&[(0, 1), (1, 3), (2, 3)]));
    assert_eq!(toggle(&mut c, 1, 3, 90, 30, (SIG, 0.0)), None);
    assert_eq!(cells(&c), on(&[(0, 1), (2, 3)]));
    // Every hit of the pad inside the step goes, on the grid or not.
    c.events.push(drum(2, 100));
    tap(&mut c, 2, 3);
    assert_eq!(cells(&c), on(&[(0, 1)]));
}

#[test]
fn fills_replace_the_row_with_every_nth_step() {
    for (beats, steps) in [(4.0, 16u32), (8.0, 32)] {
        let mut c = clip(beats);
        tap(&mut c, 0, 1);
        tap(&mut c, 1, 1);
        c.events.push(drum(1, 3 * STEP_TICKS + 5));
        for every in [1u32, 2, 4] {
            fill(&mut c, 1, every as usize, SIG);
            let row: Vec<u32> = cells(&c).iter().filter(|h| h.0 == 1).map(|h| h.1).collect();
            assert_eq!(row, (0..steps).step_by(every as usize).collect::<Vec<_>>());
            // The other row stays.
            assert!(cells(&c).contains(&(0, 1, 0)));
            assert!(c.events.is_sorted_by_key(Event::pos));
        }
    }
}

/// Fill first deletes all of the row's hits, past the loop too, then adds one on every N-th of llround(loopLen·d)
/// steps, 0 long in step mode, at velocity 255. At d = 3 a 2.1-beat loop has llround(6.3) = 6 steps, not ceil's 7.
#[test]
fn fill_follows_round_11_at_3_steps_a_beat() {
    let sig = TimeSig {
        subdivision: 3,
        beats_per_bar: 4,
    };
    let fills: [(usize, &[u32]); 3] = [
        (1, &[0, 43, 85, 128, 171, 213]),
        (2, &[0, 85, 171]),
        (4, &[0, 171]),
    ];
    for (every, starts) in fills {
        let mut c = clip(2.1);
        c.events = vec![drum(0, 50), drum(1, 600)];
        fill(&mut c, 1, every, sig);
        let row: Vec<_> = (c.events.iter())
            .filter_map(|e| match *e {
                Event::Drum {
                    pad: 1,
                    pos,
                    len_beats,
                    velocity,
                    ..
                } => Some((pos, len_beats, velocity)),
                _ => None,
            })
            .collect();
        let want: Vec<_> = starts.iter().map(|&pos| (pos, 0.0, 255)).collect();
        assert_eq!(row, want, "every {every}");
        assert!(
            c.events.iter().any(|e| e.pos() == 50),
            "the other row stays"
        );
        assert!(c.events.is_sorted_by_key(Event::pos));
    }
}

#[test]
fn clear_empties_only_its_row() {
    let mut c = clip(4.0);
    fill(&mut c, 0, 4, SIG);
    fill(&mut c, 2, 2, SIG);
    clear(&mut c, 2);
    assert_eq!(cells(&c), on(&[(0, 0), (0, 4), (0, 8), (0, 12)]));
}

/// `e` at velocity `v`.
fn loud(mut e: Event, v: u16) -> Event {
    Prop::Velocity.set(&mut e, v);
    e
}

#[test]
fn rotate_acts_on_llround_l_d_steps_and_recreates_each_occupied_step_at_its_start() {
    // Rotate: n = llround(L·d); every hit of the row goes and each occupied step i comes back at ((i ± 1) mod n)/d,
    // keeping its length and values; the step of a hit is llround(start·d), several on one step collapse to one.
    let mut c = clip(4.0);
    tap(&mut c, 1, 0);
    tap(&mut c, 0, 15);
    tap(&mut c, 1, 15);
    // 5 ticks into step 2 (llround: step 2), and 20 ticks in (llround: step 3).
    c.events.push(drum(1, 2 * STEP_TICKS + 5));
    c.events.push(loud(drum(1, 2 * STEP_TICKS + 20), 9));
    c.events.sort_by_key(Event::pos);
    rotate(&mut c, 1, true, (SIG, 0.0));
    assert_eq!(
        cells(&c),
        [(1, 0, 0), (1, 1, 0), (1, 3, 0), (1, 4, 0), (0, 15, 0)],
        "15 wraps to 0, each comes back on its step's start, row 0 stays"
    );
    assert!(c.events.contains(&loud(drum(1, 4 * STEP_TICKS), 9)));
    rotate(&mut c, 1, false, (SIG, 0.0));
    rotate(&mut c, 1, false, (SIG, 0.0));
    assert_eq!(
        cells(&c),
        [(1, 1, 0), (1, 2, 0), (1, 14, 0), (0, 15, 0), (1, 15, 0)]
    );
    // Two hits on one step (by llround) collapse to the last in clip order.
    let mut c = clip(4.0);
    c.events = vec![
        drum(1, 3 * STEP_TICKS - 10),
        loud(drum(1, 3 * STEP_TICKS), 7),
    ];
    rotate(&mut c, 1, true, (SIG, 0.0));
    assert_eq!(c.events, [loud(drum(1, 4 * STEP_TICKS), 7)]);
    // n follows the loop, however long: a 200-beat loop rotates over 800 steps, step 799 wrapping to 0.
    let mut c = clip(200.0);
    c.events = vec![drum(1, 799 * STEP_TICKS), drum(1, 600 * STEP_TICKS)];
    rotate(&mut c, 1, true, (SIG, 0.0));
    assert_eq!(cells(&c), [(1, 0, 0), (1, 601, 0)]);
}

#[test]
fn rotate_reapplies_the_swing_and_onto_the_last_step_the_refit_doubles_the_loop() {
    // Swing 50 %: an odd step starts 8 ticks late. Step 14 rotated right lands on step 15, the loop's last: it ends 8
    // ticks past the bar, and the refit doubles the loop.
    let mut c = clip(4.0);
    tap(&mut c, 1, 14);
    tap(&mut c, 1, 5);
    rotate(&mut c, 1, true, (SIG, 0.5));
    assert_eq!(
        c.events.iter().map(Event::pos).collect::<Vec<_>>(),
        [6 * STEP_TICKS, 15 * STEP_TICKS + 8]
    );
    c.fit_loop(SIG.bar_beats());
    assert_eq!(c.loop_len_beats, 8.0);
}

#[test]
fn fill_writes_steps_below_llround_l_d_only_0_long_so_it_never_grows_the_loop() {
    // A 200-beat loop is 800 steps: Fill stops at the 512-step cap; a 4.9-beat one fills llround(19.6) = 20 steps.
    let cap: Vec<u32> = (0..512).step_by(4).map(|s| s * STEP_TICKS).collect();
    let mut c = clip(200.0);
    c.events = vec![drum(1, 600 * STEP_TICKS)];
    fill(&mut c, 1, 4, SIG);
    assert_eq!(c.events.iter().map(Event::pos).collect::<Vec<_>>(), cap);
    let mut c = clip(4.9);
    fill(&mut c, 1, 1, SIG);
    assert_eq!(c.events.len(), 20);
    let mut c = clip(4.0);
    fill(&mut c, 1, 1, SIG);
    assert!(
        c.events
            .iter()
            .all(|e| matches!(e, Event::Drum { len_beats, .. } if *len_beats == 0.0))
    );
    c.fit_loop(SIG.bar_beats());
    assert_eq!(c.loop_len_beats, 4.0);
}

#[test]
fn clone_and_delete_shift_the_rows_of_every_drum_clip() {
    let mut channel = drums();
    let lane = &mut channel.lanes[0];
    lane.add_clip(0, 4.0).events = vec![drum(0, 0), drum(1, 32), drum(2, 64)];
    lane.add_clip(512, 4.0).events = vec![drum(1, 96)];
    let mut notes = Lane::new(LaneKind::Notes);
    notes.add_clip(0, 4.0).events = vec![Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 1,
        velocity: 1,
        pan: 1,
        slide: false,
    }];
    channel.lanes.push(notes.clone());

    clone_row(&mut channel.lanes, 1);
    let clips = &channel.lanes[0].clips;
    assert_eq!(cells(&clips[0]), on(&[(0, 0), (1, 1), (2, 1), (3, 2)]));
    assert_eq!(cells(&clips[1]), on(&[(1, 3), (2, 3)]));
    // The copy keeps the hit's values.
    assert_eq!(clips[1].events[1], drum(2, 96));
    assert_eq!(channel.lanes[1], notes, "other lanes stay");

    delete_row(&mut channel.lanes, 0);
    let clips = &channel.lanes[0].clips;
    assert_eq!(cells(&clips[0]), on(&[(0, 1), (1, 1), (2, 2)]));
    assert_eq!(cells(&clips[1]), on(&[(0, 3), (1, 3)]));
    assert_eq!(channel.lanes[1], notes);
}

#[test]
fn rows_cover_the_kit_and_every_pad_in_use() {
    let mut lane = Lane::new(LaneKind::Drums);
    assert_eq!(row_count(&lane, 0), 0);
    assert_eq!(row_count(&lane, 4), 4);
    lane.add_clip(0, 4.0).events = vec![drum(6, 0)];
    assert_eq!(row_count(&lane, 4), 7);
    assert_eq!(row_count(&lane, 9), 9);
}

#[test]
fn every_hit_has_its_cell_on_any_step() {
    let mut c = clip(4.0);
    // The grid has no end: hits past the loop and on any pad have their cells.
    c.events = vec![
        drum(1, 33),
        drum(0, 480),
        drum(5, 0),
        drum(1, 512),
        drum(1, 40_000),
    ];
    let marks = cells_of(&c);
    assert_eq!(marks, [(0, 15), (1, 1), (1, 16), (1, 1250), (5, 0)]);
    // Of two hits in a cell, the last.
    assert_eq!(edit::cells(&c, SIG)[&(1, 1)], &drum(1, 33));
    let mut later = drum(1, 34);
    Prop::Velocity.set(&mut later, 7);
    c.events.insert(1, later);
    assert_eq!(edit::cells(&c, SIG)[&(1, 1)], &later);
}

/// The cells [`edit::cells`] marks, sorted.
fn cells_of(c: &Clip) -> Vec<(u8, usize)> {
    let mut marks: Vec<_> = edit::cells(c, SIG).into_keys().collect();
    marks.sort_unstable();
    marks
}

#[test]
fn graph_values_go_to_the_rows_hits_in_the_step() {
    let mut c = clip(4.0);
    // Pad 1 in steps 3, 4 (twice, once off the grid line) and 9; pad 0 in step 4.
    c.events = vec![
        drum(1, 96),
        drum(0, 128),
        drum(1, 128),
        drum(1, 133),
        drum(1, 288),
    ];
    set_prop(&mut c, 1, 4, Prop::Repeat, 3, SIG);
    set_prop(&mut c, 1, 9, Prop::Pitch, 0x9000, SIG);
    let values: Vec<(u8, u16, u8)> = c
        .events
        .iter()
        .map(|e| match *e {
            Event::Drum {
                pad, value, repeat, ..
            } => (pad, value, repeat),
            _ => panic!("not a drum hit"),
        })
        .collect();
    assert_eq!(
        values,
        [(1, 7, 0), (0, 7, 0), (1, 7, 3), (1, 7, 3), (1, 0x9000, 0)]
    );
}

// The row menu.

fn menu_cells(menu: &[PopupItem<RowItem>]) -> Vec<(usize, usize, &str, bool, bool)> {
    menu.iter()
        .map(|i| (i.col, i.row, &*i.label, i.disabled, i.keep_open))
        .collect()
}

#[test]
fn the_row_menu_is_the_spec_table() {
    let menu = row_menu(4, Some(36));
    assert_eq!(
        menu_cells(&menu),
        [
            (0, 0, "Load Sample", false, false),
            (0, 1, "Load Drumset", false, false),
            (0, 2, "Save Drumset", false, false),
            (0, 3, "Clone", false, false),
            (0, 4, "Delete", false, false),
            (0, 5, "Reset MIDI Note", false, false),
            (0, 6, "Set Name", false, false),
            (1, 0, "Fill every step", false, false),
            (1, 1, "Fill each 2 step", false, false),
            (1, 2, "Fill each 4 step", false, false),
            (1, 3, "Clear", false, false),
            (1, 4, "Rotate right", false, true),
            (1, 5, "Rotate left", false, true),
        ]
    );
}

#[test]
fn reset_midi_note_needs_a_note_and_delete_a_second_row() {
    let menu = row_menu(1, None);
    let labels: Vec<&str> = menu
        .iter()
        .filter(|i| i.col == 0)
        .map(|i| &*i.label)
        .collect();
    assert_eq!(
        labels,
        [
            "Load Sample",
            "Load Drumset",
            "Save Drumset",
            "Clone",
            "Delete",
            "Set MIDI Note",
            "Set Name"
        ]
    );
    let disabled = |menu: &[PopupItem<RowItem>], label: &str| {
        menu.iter().find(|i| i.label == label).unwrap().disabled
    };
    assert!(disabled(&menu, "Delete"));
    assert!(!disabled(&row_menu(2, None), "Delete"));
    // No pad for a copy once the 22 pads are in use.
    assert!(disabled(&row_menu(PAD_COUNT, None), "Clone"));
    assert!(!disabled(&row_menu(PAD_COUNT - 1, None), "Clone"));
}

// Layout.

fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE_INDEX)
}

#[test]
fn the_row_pitch_is_a_zoom_clamped_to_2_to_100_24_u() {
    assert_eq!(clamp_pitch(1.0), 2.0);
    assert_eq!(clamp_pitch(3.0), 3.0);
    assert_eq!(clamp_pitch(5.0), 100.0 / 24.0);
    assert_eq!(
        RowView::default(),
        RowView {
            pitch: 2.0,
            scroll: 0.0
        },
        "a new clip asks for 1 U"
    );
    // A pinch over the rows zooms them within that range, around the pointer (here the rows' top).
    let mut rig = Rig::new(pattern());
    let top = pos2(400.0, 48.0);
    rig.pinch(top, 1.5);
    assert_eq!(
        rig.row_view(),
        RowView {
            pitch: 3.0,
            scroll: 0.0
        }
    );
    rig.pinch(top, 10.0);
    assert_eq!(rig.row_view().pitch, 100.0 / 24.0);
    rig.pinch(top, 0.4);
    assert_eq!(rig.row_view().pitch, 2.0);
    assert_eq!(rig.layout().pitch, 64.0);
}

#[test]
fn rows_take_the_zoom_pitch_and_steps_share_the_width() {
    // U = 32: the strip is 48 tall, the headers 96 wide, a new clip's rows 64 apart.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 448.0));
    let l = Layout::new(rect, 4, 16, &metrics(), 0.0, RowView::default());
    assert_eq!(
        l.swing_strip(),
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 48.0))
    );
    // Pitch 64, step 43.875 (702 points from the t gap, 2 points), each one pixel short.
    assert_eq!(
        l.header(0),
        Rect::from_min_max(pos2(0.0, 48.0), pos2(96.0, 111.0))
    );
    assert_eq!(
        l.cell(1, 2),
        Rect::from_min_max(pos2(186.0, 112.0), pos2(229.0, 175.0))
    );
    assert_eq!(l.hit(pos2(187.0, 113.0)), Some((1, 2)));
    assert_eq!(l.hit(pos2(799.0, 303.0)), Some((3, 15)));
    assert_eq!(l.hit(pos2(799.0, 305.0)), None, "below the last row");
    assert_eq!(l.hit(pos2(50.0, 113.0)), None, "a header");
    assert_eq!(l.hit(pos2(185.0, 20.0)), None, "the strip");
    let l = Layout::new(rect, 4, 32, &metrics(), 0.0, RowView::default());
    assert_eq!(l.cell(0, 31).x_range(), Rangef::new(778.0, 799.0));
    assert_eq!(l.hit(pos2(780.0, 60.0)), Some((0, 31)));
    // The drum mixer's room sits between the headers, which keep their 3 U, and the steps.
    let l = Layout::new(rect, 4, 16, &metrics(), 256.0, RowView::default());
    assert_eq!(l.header(0).x_range(), Rangef::new(0.0, 96.0));
    assert_eq!(l.cell(0, 0).left(), 354.0);
    assert_eq!(l.hit(pos2(200.0, 60.0)), None, "the mixer");
}

#[test]
fn the_scroll_range_is_the_rows_and_one_and_a_half_more() {
    // 12 rows of 64 and 96 of room in a 400-tall body: 464 to scroll, 14.5 U.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 448.0));
    let at = |scroll| {
        Layout::new(
            rect,
            12,
            16,
            &metrics(),
            0.0,
            RowView { pitch: 2.0, scroll },
        )
    };
    assert_eq!(at(100.0).view.scroll, 14.5);
    assert_eq!(at(-3.0).view.scroll, 0.0);
    // Header i spans gridTop + i pitch - scroll, one pixel short of the pitch; the cells follow.
    let l = at(14.5);
    assert_eq!(
        l.header(11),
        Rect::from_min_max(pos2(0.0, 288.0), pos2(96.0, 351.0))
    );
    assert_eq!(l.cell(11, 0).y_range(), Rangef::new(288.0, 351.0));
    assert_eq!(l.hit(pos2(300.0, 340.0)), Some((11, 4)));
    assert_eq!(l.hit(pos2(300.0, 60.0)), Some((7, 4)));
    // The `+`: 2 U square, 0.75 U below the last row, centred in the header cell; it ends 0.25 U above the range's
    // end, the body's bottom.
    assert_eq!(
        l.add_button(),
        Rect::from_min_size(pos2(16.0, 376.0), vec2(64.0, 64.0))
    );
    // Rows that fit do not scroll.
    let l = Layout::new(
        rect,
        4,
        16,
        &metrics(),
        0.0,
        RowView {
            pitch: 2.0,
            scroll: 1.0,
        },
    );
    assert_eq!(l.view.scroll, 0.0);
}

#[test]
fn edges_fall_on_device_pixels() {
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(801.0, 500.0));
    let l = Layout::new(
        rect,
        12,
        16,
        &Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX),
        0.0,
        RowView {
            pitch: 2.3,
            scroll: 0.37,
        },
    );
    for r in [l.cell(1, 7), l.header(2), l.add_button()] {
        for v in [r.left(), r.right(), r.top(), r.bottom()] {
            assert_eq!((v * 2.0).fract(), 0.0, "{r:?}");
        }
    }
}

// Offscreen egui: the view in a real context, driven by synthetic input. Each action is applied to the channel.

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(800.0, 600.0),
};
const LOCK: Pos2 = pos2(700.0, 24.0);
/// The channel under test.
const CH: usize = 1;
/// The `SWING` button: the top strip's header cell.
const SWING: Pos2 = pos2(48.0, 24.0);
/// The popup's row height at U = 32.
const MENU_ROW: f32 = 40.0;

struct Rig {
    ctx: Context,
    view: StepSeqView,
    /// The master, then the channel under test at [`CH`].
    song: Song,
    kit: Kit,
    time: f64,
    actions: Vec<StepAction>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new(channel: Channel) -> Self {
        let mut song = Song::default();
        song.channels.push(channel);
        let mut rig = Self {
            ctx: Context::default(),
            // On the lane's first clip, as the app opens it ([`crate::ui::views::Slots::follow`]); none without one.
            view: StepSeqView {
                clip: song.channels[CH]
                    .lanes
                    .first()
                    .and_then(|l| l.clips.first())
                    .map(|c| c.id),
                ..StepSeqView::default()
            },
            song,
            kit: kit(),
            time: 0.0,
            actions: Vec::new(),
            shapes: Vec::new(),
        };
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (view, song, kit) = (&mut self.view, &self.song, &self.kit);
        let mut action = None;
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = view.show(ui, &cx, song, CH, kit);
                    view.graph_knob(ui, &cx, SCREEN);
                    view.lock_button(ui, &cx, LOCK);
                });
        });
        self.shapes = output.shapes;
        if let Some(action) = action {
            self.view.apply(&mut self.song, CH, &action);
            self.actions.push(action);
        }
    }

    fn press(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn click_with(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.press(pos, button, true);
        self.press(pos, button, false);
        // Lets a popup opened by the click lay itself out.
        self.frame(vec![]);
    }

    fn click(&mut self, pos: Pos2) {
        self.click_with(pos, PointerButton::Primary);
    }

    fn key(&mut self, key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![event(true), event(false)]);
    }

    fn channel(&self) -> &Channel {
        &self.song.channels[CH]
    }

    /// The layout the view drew its last frame with.
    fn layout(&self) -> Layout {
        self.view.last.expect("the view drew")
    }

    /// The shown clip's view.
    fn clip_view(&self) -> ClipView {
        let key = self.view.key(CH, &self.channel().lanes[self.view.lane]);
        self.view.views[&key]
    }

    /// The shown clip's rows' zoom and scroll.
    fn row_view(&self) -> RowView {
        self.clip_view().rows
    }

    /// Sets the shown clip's rows' zoom and scroll, and draws a frame.
    fn set_row_view(&mut self, rows: RowView) {
        let key = self.view.key(CH, &self.channel().lanes[self.view.lane]);
        let view = ClipView {
            rows,
            ..self.clip_view()
        };
        self.view.views.insert(key, view);
        self.frame(vec![]);
    }

    /// A pinch by `factor` with the pointer at `at`.
    fn pinch(&mut self, at: Pos2, factor: f32) {
        // Two fingers: the first stays on `at`, the anchor; the second, below it, spreads (or closes) the separation by
        // `factor`. (The trackpad's pinch is the time's, which the steps do not have.)
        let (from, to) = if factor >= 1.0 {
            (100.0, 100.0 * factor)
        } else {
            (100.0 / factor, 100.0)
        };
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        use egui::TouchPhase::{End, Move, Start};
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![touch(1, Start, at), button(true)]);
        self.frame(vec![touch(2, Start, at + vec2(0.0, from))]);
        self.frame(vec![touch(2, Move, at + vec2(0.0, to))]);
        self.frame(vec![
            touch(2, End, at + vec2(0.0, to)),
            touch(1, End, at),
            button(false),
            egui::Event::PointerGone,
        ]);
        self.frame(vec![]);
    }

    /// Scrolls the shown clip's time to content beat `x` and draws a frame (the frame clamps it to its range).
    fn scroll_to(&mut self, x: f64) {
        let key = self.view.key(CH, &self.channel().lanes[self.view.lane]);
        let view = ClipView {
            x,
            ..self.clip_view()
        };
        self.view.views.insert(key, view);
        self.frame(vec![]);
    }

    /// A vertical drag by `dy` from `from`, in four moves, held still before the release (no fling).
    fn drag(&mut self, from: Pos2, dy: f32) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.press(from, PointerButton::Primary, true);
        for i in 1..=4 {
            let at = from + vec2(0.0, dy * i as f32 / 4.0);
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
        self.frame(vec![]);
        self.press(from + vec2(0.0, dy), PointerButton::Primary, false);
        self.frame(vec![]);
    }

    /// The texts whose left-centre lies in `header`, in paint order: (text, galley rect, font size, clip rect).
    fn header_lines(&self, header: Rect) -> Vec<(String, Rect, f32, Rect)> {
        self.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(s) => Some((
                    s.galley.text().to_owned(),
                    Rect::from_min_size(s.pos, s.galley.size()),
                    s.galley.job.sections[0].format.font_id.size,
                    c.clip_rect,
                )),
                _ => None,
            })
            .filter(|b| header.contains(b.1.left_center()))
            .collect()
    }

    fn cell(&self, row: usize, step: usize) -> Pos2 {
        self.layout().cell(row, step).center()
    }

    fn header(&self, row: usize) -> Pos2 {
        self.layout().header(row).center()
    }

    fn hits(&self) -> Vec<(u8, u32, u32)> {
        self.channel().lanes[0]
            .clips
            .first()
            .map_or_else(Vec::new, cells)
    }

    fn area(&self, name: &str) -> Option<Rect> {
        self.ctx.memory(|m| m.area_rect(id().with(name)))
    }

    /// Centre of item (`col`, `row`) of the open popup `name`.
    fn item(&self, name: &str, col: usize, row: usize) -> Pos2 {
        let r = self.area(name).expect("popup open");
        let x = if col == 0 {
            r.left() + 10.0
        } else {
            r.right() - 10.0
        };
        pos2(x, r.top() + (row as f32 + 0.5) * MENU_ROW)
    }

    /// Long-presses row `row`'s header and picks (`col`, `item`) in its menu.
    fn menu(&mut self, row: usize, col: usize, item: usize) {
        let header = self.header(row);
        self.click_with(header, PointerButton::Secondary);
        assert!(matches!(self.view.overlay, Some(Overlay::Menu { .. })));
        let at = self.item("row menu", col, item);
        self.click(at);
    }

    /// Every text painted in the last frame, with its colour.
    fn texts(&self) -> Vec<(String, Color32)> {
        self.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections[0].format.color,
                )),
                _ => None,
            })
            .collect()
    }

    /// Drags the swing slider by `f` of its range: its thumb follows the pointer along the 4.5 U (144-point) track.
    fn drag_swing(&mut self, f: f32) {
        let dx = f * 144.0;
        let start = self.area("swing").expect("swing popup open").center();
        self.frame(vec![egui::Event::PointerMoved(start)]);
        self.press(start, PointerButton::Primary, true);
        for i in 1..=4 {
            let at = start + vec2(dx * i as f32 / 4.0, 0.0);
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
        self.press(start + vec2(dx, 0.0), PointerButton::Primary, false);
        self.frame(vec![]);
    }

    /// Rects painted in `color`.
    fn rects(&self, color: Color32) -> Vec<Rect> {
        self.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == color => Some(r.rect),
                _ => None,
            })
            .collect()
    }
}

/// A Drums channel with a one-bar clip: Kick on 0, Snare on 4 and 12, Hat on 2.
fn pattern() -> Channel {
    let mut channel = drums();
    let c = channel.lanes[0].add_clip(0, 4.0);
    for (pad, step) in [(0, 0), (2, 2), (1, 4), (1, 12)] {
        tap(c, pad, step);
    }
    channel
}

#[test]
fn a_tap_on_a_step_toggles_it_in_the_song() {
    let mut rig = one_bar();
    let at = rig.cell(1, 2);
    rig.click(at);
    // In the one clip, still one bar.
    assert_eq!(rig.channel().lanes[0].clips.len(), 1);
    assert_eq!(rig.channel().lanes[0].clips[0].loop_len_beats, 4.0);
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 1, step: 2 }]);
    assert_eq!(rig.hits(), on(&[(1, 2)]));
    let at = rig.cell(3, 15);
    rig.click(at);
    assert_eq!(rig.hits(), on(&[(1, 2), (3, 15)]));
    let at = rig.cell(1, 2);
    rig.click(at);
    assert_eq!(rig.hits(), on(&[(3, 15)]));
}

#[test]
fn a_two_bar_clip_scrolls_to_its_32nd_step() {
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 8.0);
    let mut rig = Rig::new(channel);
    // At 3.03 U a beat the 702-point grid shows 7.24 beats: step 31 (beat 7.75) is in view from beat 4.
    rig.scroll_to(4.0);
    let at = rig.cell(0, 31);
    rig.click(at);
    assert_eq!(rig.hits(), on(&[(0, 31)]));
}

#[test]
fn a_tap_on_a_row_header_selects_the_row() {
    let mut rig = Rig::new(pattern());
    assert_eq!(rig.view.selected, 0);
    let at = rig.header(2);
    rig.click(at);
    assert_eq!(rig.view.selected, 2);
    assert!(rig.actions.is_empty());
    assert_eq!(rig.view.overlay, None);
}

// The drum mixer's fold.

#[test]
fn the_fold_moves_0_4_of_the_way_per_60_hz_frame_and_snaps_within_0_05() {
    let frame = 1.0 / 60.0;
    let run = |from: f32, to: f32| {
        let (mut c, mut seen) = (from, Vec::new());
        while c != to {
            c = fold_step(c, to, frame);
            seen.push(c);
        }
        seen
    };
    let near = |a: &[f32], b: &[f32]| {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5)
    };
    let out = run(0.0, 1.0);
    assert!(
        near(&out, &[0.4, 0.64, 0.784, 0.8704, 0.92224, 1.0]),
        "{out:?}"
    );
    let back = run(1.0, 0.0);
    assert!(
        near(&back, &[0.6, 0.36, 0.216, 0.1296, 0.07776, 0.0]),
        "{back:?}"
    );
    // By time, not by frame: two frames' time is two steps, none is none.
    assert!((fold_step(0.0, 1.0, 2.0 * frame) - 0.64).abs() < 1e-5);
    assert_eq!(fold_step(0.3, 1.0, 0.0), 0.3);
    // Stepped by input time: a frame run again at the same time steps nothing, and the first frame after a pause
    // steps one (egui's stable dt) rather than jumping to the end.
    let mut fold = Fold {
        open: true,
        at: 10.0,
        ..Fold::default()
    };
    assert!(fold.step(10.0 + f64::from(frame), frame));
    assert!((fold.c - 0.4).abs() < 1e-5);
    fold.step(10.0 + f64::from(frame), frame);
    assert!((fold.c - 0.4).abs() < 1e-5, "a re-run");
    fold.step(100.0, frame);
    assert!((fold.c - 0.64).abs() < 1e-5, "after a pause");
    for _ in 0..4 {
        fold.step(fold.at + f64::from(frame), frame);
    }
    assert_eq!(fold.c, 1.0);
    assert!(!fold.step(fold.at + f64::from(frame), frame), "at rest");
}

#[test]
fn a_tap_on_the_current_rows_header_toggles_the_mixer_and_on_another_only_selects() {
    let mut rig = Rig::new(pattern());
    // Each run starts collapsed: the header column is 3 U.
    assert_eq!(
        (rig.view.fold.open, rig.view.fold.c, rig.view.mixer_w),
        (false, 0.0, 0.0)
    );
    assert_eq!(rig.layout().grid_left(), 3.0 * 32.0 + 2.0);
    // Another row's header: it only becomes current.
    let at = rig.header(2);
    rig.click(at);
    assert_eq!((rig.view.selected, rig.view.fold.open), (2, false));
    // The current row's: the mixer folds out, the column following every frame to 3 U + 12 U.
    rig.click(at);
    assert!(rig.view.fold.open);
    rig.frame(vec![]);
    assert_eq!(rig.view.fold.c, 1.0);
    assert_eq!(rig.view.mixer_w, 12.0 * 32.0);
    assert_eq!(rig.layout().grid_left(), 15.0 * 32.0 + 2.0);
    let at = rig.header(0);
    rig.click(at);
    assert_eq!((rig.view.selected, rig.view.fold.open), (0, true));
    rig.click(at);
    assert!(!rig.view.fold.open);
    rig.frame(vec![]);
    assert_eq!((rig.view.fold.c, rig.view.mixer_w), (0.0, 0.0));
    // A long press keeps the row menu, and toggles nothing.
    rig.click_with(at, PointerButton::Secondary);
    assert!(matches!(
        rig.view.overlay,
        Some(Overlay::Menu { row: 0, .. })
    ));
    assert!(!rig.view.fold.open);
    assert!(rig.actions.is_empty());
}

#[test]
fn a_drag_from_the_current_rows_header_scrolls_and_does_not_fold_the_mixer() {
    // "Dragging does not resize": a drag that starts on the current header is the rows' scroll.
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    let at = rig.header(0);
    rig.drag(at, -200.0);
    assert_eq!(rig.row_view().scroll, 200.0 / 32.0);
    assert_eq!((rig.view.fold.open, rig.view.fold.c), (false, 0.0));
    assert!(rig.actions.is_empty());
}

#[test]
fn the_current_header_is_channel_filled_with_module_fill_texts_the_others_channel_texts() {
    let rig = Rig::new(pattern());
    let theme = theme_by_id(0);
    let ink = channel_color(rig.channel().hue, &theme).to_color32();
    let module = theme.colors.module_fill.to_color32();
    let l = rig.layout();
    assert!(rig.rects(ink).contains(&l.header(0)));
    assert!(!rig.rects(ink).contains(&l.header(1)));
    let color = |name: &str| rig.texts().into_iter().find(|t| t.0 == name).unwrap().1;
    for (row, (pad, sample, _)) in kit().0.into_iter().enumerate() {
        let want = if row == 0 { module } else { ink };
        assert_eq!((color(pad), color(sample)), (want, want), "row {row}");
    }
}

#[test]
fn with_the_mixer_out_each_rows_strip_is_module_fill() {
    // The strip row is `moduleFill`, so an off strip disc is unseen on it.
    let mut rig = Rig::new(pattern());
    let module = theme_by_id(0).colors.module_fill.to_color32();
    let strip = |rig: &Rig, row: usize| {
        let (l, header) = (rig.layout(), rig.layout().header(row));
        Rect::from_x_y_ranges(l.header_right()..=l.column_right(), header.y_range())
    };
    assert!(
        !rig.rects(module).contains(&strip(&rig, 0)),
        "folded in: none"
    );
    let at = rig.header(0);
    rig.click(at);
    rig.frame(vec![]);
    assert_eq!(rig.view.fold.c, 1.0);
    for row in 0..kit().0.len() {
        let want = strip(&rig, row);
        assert_eq!(want.width(), 12.0 * 32.0);
        assert!(rig.rects(module).contains(&want), "row {row}: {want:?}");
    }
}

#[test]
fn the_current_header_grows_up_to_10_px_while_held_and_wobbles_back() {
    // The knobs' touch pulse (`widgets::pulse_growth`): q = 0.6 per 60 Hz frame from 1, the touch-down and release
    // frames included; f = 1 − q while held, (1 − q)·sin((1 − q)·2π) after; the growth is 10 px · f.
    let mut rig = Rig::new(pattern());
    let ink = channel_color(rig.channel().hue, &theme_by_id(0)).to_color32();
    let h = rig.layout().header(0);
    let fill = |rig: &Rig| {
        (rig.rects(ink).into_iter())
            .find(|r| r.center() == h.center())
            .unwrap()
    };
    let growth = |rig: &Rig| fill(rig).height() - h.height();
    rig.frame(vec![egui::Event::PointerMoved(h.center())]);
    rig.press(h.center(), PointerButton::Primary, true);
    assert!((growth(&rig) - 4.0).abs() < 1e-3, "{}", growth(&rig));
    rig.frame_after(1.0 / 60.0, vec![]);
    assert!((growth(&rig) - 6.4).abs() < 1e-3, "{}", growth(&rig));
    // Held half a second: 9.9 px larger each way about its centre.
    rig.frame_after(0.5, vec![]);
    let grown = fill(&rig);
    assert!(
        (grown.size() - h.size() - vec2(9.9, 9.9)).length() < 1e-3,
        "{grown:?} from {h:?}"
    );
    // Painted over the steps it grows onto.
    let at = |want: &dyn Fn(Rect) -> bool| {
        (rig.shapes.iter()).position(|s| matches!(&s.shape, egui::Shape::Rect(r) if want(r.rect)))
    };
    let cell = rig.layout().cell(0, 0);
    assert!(at(&|r| r == cell) < at(&|r| r == grown));
    // Let go, it wobbles (over and under its size) and settles back. Held past 300 ms it was a long press (the row
    // menu), no tap: the mixer stays in.
    rig.press(h.center(), PointerButton::Primary, false);
    let mut wobble = vec![growth(&rig)];
    for _ in 0..8 {
        rig.frame_after(1.0 / 60.0, vec![]);
        wobble.push(growth(&rig));
    }
    assert!(
        wobble.iter().any(|&g| g > 2.0) && wobble.iter().any(|&g| g < -4.0),
        "{wobble:?}"
    );
    rig.frame_after(0.5, vec![]);
    assert!(rig.rects(ink).contains(&h));
    assert!(!rig.view.fold.open);
}

#[test]
fn level_meters_stand_at_the_headers_right_edge_until_the_mixer_is_fully_out() {
    let t = 2.0;
    let header = Rect::from_min_max(pos2(0.0, 48.0), pos2(96.0, 111.0));
    let page = Rect::from_min_max(pos2(96.0, 48.0), pos2(480.0, 111.0));
    // Collapsed or folding: two thin vertical bars at the header's right edge, in the 5 hairlines its text leaves,
    // filling from the bottom.
    let [l, r] = meter_bars(header, page, false, [1.0, 0.5], t);
    assert_eq!(l, Rect::from_min_max(pos2(86.0, 48.0), pos2(90.0, 111.0)));
    assert_eq!(r, Rect::from_min_max(pos2(92.0, 79.5), pos2(96.0, 111.0)));
    // Fully out: horizontal bars at the bottom of the page area, filling from the left.
    let [l, r] = meter_bars(header, page, true, [1.0, 0.5], t);
    assert_eq!(l, Rect::from_min_max(pos2(96.0, 101.0), pos2(480.0, 105.0)));
    assert_eq!(r, Rect::from_min_max(pos2(96.0, 107.0), pos2(288.0, 111.0)));
    // At rest (silence: no meter feed yet) both are empty, either way.
    for full in [false, true] {
        assert!(
            meter_bars(header, page, full, [0.0; 2], t)
                .iter()
                .all(|b| !b.is_positive())
        );
    }
}

#[test]
fn swing_has_the_3_u_ruler_cell_to_itself_in_step_sequencer_mode() {
    // `SWING` 3 U × (1.5 U − 1 px) at the cell's top left. Snap belongs to the free note-grid view (a notes lane); this
    // step-sequencer view never draws it or takes touches for it.
    let rig = Rig::new(pattern());
    let swing = rig
        .ctx
        .read_response(id().with("swing button"))
        .unwrap()
        .rect;
    assert_eq!(swing, Rect::from_min_size(pos2(0.0, 0.0), vec2(96.0, 47.0)));
    let cell = Rect::from_min_size(pos2(0.0, 0.0), vec2(96.0, 48.0));
    let texts: Vec<String> = (rig.shapes.iter())
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if cell.contains(t.visual_bounding_rect().center()) => {
                Some(t.galley.text().to_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["SWING"]);
}

#[test]
fn the_selected_header_is_channel_coloured_and_the_hits_take_the_hit_colour() {
    let mut rig = Rig::new(pattern());
    let at = rig.header(2);
    rig.click(at);
    let ink = channel_color(rig.channel().hue, &theme_by_id(0)).to_color32();
    let painted = rig.rects(ink);
    let l = rig.layout();
    assert!(painted.contains(&l.header(2)));
    assert!(!painted.contains(&l.header(0)));
    // One tall hit per active step, inside its cell, and a dark dash on each. The pattern's hits are velocity 255.
    let hits: Vec<Rect> = rig
        .rects(tinted(&rig, Prop::Velocity, 255, false))
        .iter()
        .copied()
        .filter(|r| l.grid().contains_rect(*r))
        .collect();
    assert_eq!(hits.len(), 4);
    for (row, step) in [(0, 0), (2, 2), (1, 4), (1, 12)] {
        let hit = hits
            .iter()
            .find(|h| l.cell(row, step).contains_rect(**h))
            .unwrap();
        assert!(hit.height() > hit.width(), "tall: {hit:?}");
    }
    let bg = theme_by_id(0).colors.background.to_color32();
    let dashes = rig
        .rects(bg)
        .into_iter()
        .filter(|d| hits.iter().any(|h| h.contains_rect(*d)))
        .count();
    assert_eq!(dashes, 4);
}

#[test]
fn a_long_press_selects_the_row_and_opens_its_menu() {
    let mut rig = Rig::new(pattern());
    let at = rig.header(3);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.view.selected, 3);
    assert!(rig.area("row menu").is_some());
    // A disabled item does nothing and keeps the menu: `Delete` on the one row of a one-pad kit.
    let mut rig = Rig::new(drums());
    rig.kit.0.truncate(1);
    rig.frame(vec![]);
    let at = rig.header(0);
    rig.click_with(at, PointerButton::Secondary);
    let at = rig.item("row menu", 0, 4);
    rig.click(at);
    assert!(rig.actions.is_empty());
    assert!(matches!(
        rig.view.overlay,
        Some(Overlay::Menu { row: 0, .. })
    ));
}

#[test]
fn the_rows_file_items_ask_the_host_and_change_nothing_here() {
    // Rows 1–3: `Load Sample` opens the sample picker for the row; `Load Drumset` the drumset picker; `Save Drumset`
    // saves the kit. The host does each.
    let mut rig = Rig::new(pattern());
    let song = rig.song.clone();
    for (item, action) in [
        (0, StepAction::LoadSample { pad: 2 }),
        (1, StepAction::LoadDrumset),
        (2, StepAction::SaveDrumset),
    ] {
        rig.actions.clear();
        rig.menu(2, 0, item);
        assert_eq!(rig.actions, [action]);
        assert_eq!(rig.view.overlay, None, "closes");
        assert_eq!(rig.song, song);
    }
}

/// The `+` add-drum button's ring (icon `AddLarge`), in `color`.
fn add_ring(rig: &Rig, color: Color32) -> bool {
    let center = rig.layout().add_button().center();
    rig.shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::Circle(c)
            if c.center.distance(center) < 0.5 && c.stroke.color == color)
    })
}

#[test]
fn the_add_drum_button_asks_the_host_until_the_pads_are_all_in_use() {
    // The `+` add-drum button, 0.75 U below the last row. The host opens the sample picker, whose pick adds the pad.
    let text = theme_by_id(0).colors.text.to_color32();
    let mut rig = Rig::new(pattern());
    assert!(add_ring(&rig, text));
    let at = rig.layout().add_button().center();
    rig.click(at);
    assert_eq!(rig.actions, [StepAction::AddDrum]);
    // With a slot on every one of the 22 pads there is no pad to add: dimmed, and a click does nothing.
    rig.kit = Kit(vec![("Pad", "", None); PAD_COUNT]);
    // Scrolled to the bottom, where the button is.
    rig.set_row_view(RowView {
        pitch: 2.0,
        scroll: 99.0,
    });
    rig.actions.clear();
    assert!(add_ring(
        &rig,
        text.gamma_multiply(crate::theme::DISABLED_ALPHA)
    ));
    let at = rig.layout().add_button().center();
    rig.click(at);
    assert_eq!(rig.actions, []);
}

#[test]
fn the_fill_and_clear_items_edit_the_row() {
    let mut rig = Rig::new(pattern());
    rig.menu(2, 1, 1);
    assert_eq!(rig.actions, [StepAction::Fill { pad: 2, every: 2 }]);
    assert_eq!(rig.view.overlay, None, "closes");
    let row = |rig: &Rig, pad| -> Vec<u32> {
        rig.hits()
            .iter()
            .filter(|h| h.0 == pad)
            .map(|h| h.1)
            .collect()
    };
    assert_eq!(row(&rig, 2), [0, 2, 4, 6, 8, 10, 12, 14]);
    rig.menu(2, 1, 2);
    assert_eq!(row(&rig, 2), [0, 4, 8, 12]);
    rig.menu(2, 1, 0);
    assert_eq!(row(&rig, 2), (0..16).collect::<Vec<_>>());
    rig.menu(2, 1, 3);
    assert_eq!(row(&rig, 2), [] as [u32; 0]);
    assert_eq!(row(&rig, 1), [4, 12], "other rows stay");
}

#[test]
fn the_rotate_items_keep_the_menu_open() {
    let mut rig = Rig::new(pattern());
    rig.menu(1, 1, 4);
    assert_eq!(
        rig.actions,
        [StepAction::Rotate {
            pad: 1,
            right: true
        }]
    );
    assert!(matches!(
        rig.view.overlay,
        Some(Overlay::Menu { row: 1, .. })
    ));
    let at = rig.item("row menu", 1, 4);
    rig.click(at);
    let at = rig.item("row menu", 1, 5);
    rig.click(at);
    let row: Vec<u32> = rig
        .hits()
        .iter()
        .filter(|h| h.0 == 1)
        .map(|h| h.1)
        .collect();
    assert_eq!(row, [5, 13]);
}

#[test]
fn clone_copies_the_row_below_and_selects_it() {
    let mut rig = Rig::new(pattern());
    rig.menu(1, 0, 3);
    assert_eq!(rig.actions, [StepAction::Clone { pad: 1 }]);
    assert_eq!(
        rig.hits(),
        on(&[(0, 0), (3, 2), (1, 4), (2, 4), (1, 12), (2, 12)])
    );
    assert_eq!(rig.view.selected, 2);
}

#[test]
fn a_quick_pinch_on_a_step_toggles_nothing() {
    use egui::{Event, TouchDeviceId, TouchId, TouchPhase};
    let touch = |id, phase, pos| Event::Touch {
        device_id: TouchDeviceId(1),
        id: TouchId(id),
        phase,
        pos,
        force: None,
    };
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let mut rig = Rig::new(pattern());
    let hits = rig.hits();
    // The first finger stays put on a step while the second one moves.
    let (at, other) = (rig.cell(0, 6), rig.cell(3, 12));
    rig.frame(vec![Event::PointerMoved(at)]);
    rig.frame(vec![touch(1, TouchPhase::Start, at), button(at, true)]);
    rig.frame(vec![touch(2, TouchPhase::Start, other)]);
    rig.frame(vec![touch(2, TouchPhase::Move, other + vec2(0.0, 20.0))]);
    rig.frame(vec![touch(2, TouchPhase::End, other + vec2(0.0, 20.0))]);
    rig.frame(vec![
        touch(1, TouchPhase::End, at),
        button(at, false),
        Event::PointerGone,
    ]);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    assert_eq!(rig.hits(), hits);
    // A tap after it still toggles.
    let at = rig.cell(0, 6);
    rig.click(at);
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 0, step: 6 }]);
}

#[test]
fn a_full_kit_has_no_pad_to_clone_into() {
    let mut rig = Rig::new(pattern());
    rig.kit = Kit(vec![("Pad", "", None); PAD_COUNT]);
    rig.frame(vec![]);
    rig.menu(1, 0, 3);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // Hits on the last pad fill the grid's rows as well.
    let mut rig = Rig::new(pattern());
    tap(&mut rig.song.channels[CH].lanes[0].clips[0], 21, 0);
    rig.frame(vec![]);
    rig.menu(1, 0, 3);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
}

#[test]
fn delete_asks_first() {
    let mut rig = Rig::new(pattern());
    rig.menu(1, 0, 4);
    assert!(rig.actions.is_empty());
    assert_eq!(rig.view.overlay, Some(Overlay::ConfirmDelete(1)));
    // `No` keeps the row.
    let at = rig.item("delete", 0, 2);
    rig.click(at);
    assert_eq!(rig.view.overlay, None);
    assert!(rig.actions.is_empty());
    // `Yes` deletes it; the rows below move up.
    rig.menu(1, 0, 4);
    let at = rig.item("delete", 0, 1);
    rig.click(at);
    assert_eq!(rig.actions, [StepAction::Delete { pad: 1 }]);
    assert_eq!(rig.hits(), on(&[(0, 0), (1, 2)]));
}

#[test]
fn set_name_prompts_for_the_custom_name() {
    // The prompt opens with the current custom name (or empty); OK with text sets it, OK with empty text removes it.
    // `New sample name` is the audio-clip prompt, not this one.
    let mut rig = Rig::new(pattern());
    // `Set Name` is row 6 whether the pad has a MIDI note (Snare) or not: the note items share row 5.
    rig.menu(1, 0, 6);
    // No custom name: an empty field.
    assert!(matches!(&rig.view.overlay, Some(Overlay::Rename { row: 1, text }) if text.is_empty()));
    rig.frame(vec![]);
    assert!(rig.texts().iter().any(|t| t.0 == "Set Name"));
    assert!(!rig.texts().iter().any(|t| t.0 == "New sample name"));
    rig.frame(vec![egui::Event::Text("Rim".into())]);
    assert!(rig.actions.is_empty());
    rig.key(egui::Key::Enter);
    assert_eq!(
        rig.actions,
        [StepAction::Rename {
            pad: 1,
            name: "Rim".into()
        }]
    );
    assert_eq!(rig.view.overlay, None);
    // Escape cancels.
    rig.menu(0, 0, 6);
    assert!(matches!(
        rig.view.overlay,
        Some(Overlay::Rename { row: 0, .. })
    ));
    rig.key(egui::Key::Escape);
    assert_eq!(rig.view.overlay, None);
    assert_eq!(rig.actions.len(), 1);
    // A pad with a custom name: the field starts from it, and OK on an empty field removes it.
    rig.kit.0[0] = ("Mine", "", None);
    rig.menu(0, 0, 6);
    assert!(matches!(&rig.view.overlay, Some(Overlay::Rename { row: 0, text }) if text == "Mine"));
    rig.frame(vec![]);
    for _ in 0.."Mine".len() {
        rig.key(egui::Key::Backspace);
    }
    rig.key(egui::Key::Enter);
    assert_eq!(
        rig.actions.last(),
        Some(&StepAction::Rename {
            pad: 0,
            name: String::new()
        })
    );
}

#[test]
fn set_midi_note_waits_for_a_key_and_reset_clears_it() {
    let mut rig = Rig::new(pattern());
    assert_eq!(rig.view.midi_key(40), None, "not waiting");
    rig.menu(2, 0, 5);
    assert_eq!(rig.view.overlay, Some(Overlay::LearnMidi(2)));
    assert!(rig.area("midi").is_some());
    assert_eq!(
        rig.view.midi_key(40),
        Some(StepAction::SetMidiNote {
            pad: 2,
            note: Some(40)
        })
    );
    assert_eq!(rig.view.overlay, None);
    // Snare has a note: `Reset MIDI Note` is row 5.
    rig.menu(1, 0, 5);
    assert_eq!(
        rig.actions,
        [StepAction::SetMidiNote { pad: 1, note: None }]
    );
    // `Cancel` stops waiting.
    rig.menu(2, 0, 5);
    let at = rig.item("midi", 0, 1);
    rig.click(at);
    assert_eq!(rig.view.overlay, None);
}

/// A Drums channel with Kick on steps 0 to 3 only.
fn sixteenths() -> Channel {
    let mut channel = drums();
    let c = channel.lanes[0].add_clip(0, 4.0);
    for step in 0..4 {
        tap(c, 0, step);
    }
    channel
}

fn positions(rig: &Rig) -> Vec<u32> {
    rig.channel().lanes[0].clips[0]
        .events
        .iter()
        .map(Event::pos)
        .collect()
}

#[test]
fn swing_is_a_button_that_opens_a_popup() {
    let mut rig = Rig::new(pattern());
    assert!(rig.area("swing").is_none());
    rig.click(SWING);
    assert!(matches!(rig.view.overlay, Some(Overlay::Swing { .. })));
    assert!(rig.area("swing").is_some());
    let texts: Vec<String> = rig.texts().into_iter().map(|t| t.0).collect();
    assert!(texts.iter().any(|t| t == "Swing"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "0%"), "{texts:?}");
    assert!(rig.actions.is_empty());
}

#[test]
fn releasing_the_swing_slider_moves_the_hits_once_from_the_snapshot() {
    let mut rig = Rig::new(sixteenths());
    rig.click(SWING);
    rig.drag_swing(1.0);
    // One edit for the whole drag, from the positions before it: offbeat sixteenths move 16 ticks at s = 1.
    assert_eq!(
        rig.actions,
        [StepAction::Swing {
            amount: 1.0,
            from: vec![0, 32, 64, 96],
            final_edit: true,
        }]
    );
    assert_eq!(positions(&rig), [0, 48, 64, 112]);
    assert_eq!(rig.channel().lanes[0].swing, 1.0);
    // A second drag in the same popup starts from the same snapshot, not from the moved hits.
    rig.drag_swing(-0.5);
    assert_eq!(rig.actions.len(), 2);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
    assert_eq!(rig.channel().lanes[0].swing, 0.5);
    // A click outside closes the popup without another edit, and does not reach the grid.
    let outside = rig.cell(2, 10);
    rig.click(outside);
    assert_eq!(rig.view.overlay, None);
    assert_eq!(rig.actions.len(), 2);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
}

#[test]
fn a_new_swing_popup_starts_from_the_moved_hits() {
    let mut rig = Rig::new(sixteenths());
    rig.click(SWING);
    rig.drag_swing(0.5);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
    rig.key(egui::Key::Escape);
    assert_eq!(rig.view.overlay, None);
    // The regular grid puts a moved hit back on its sixteenth before swinging it again.
    rig.click(SWING);
    rig.drag_swing(-0.25);
    assert_eq!(positions(&rig), [0, 36, 64, 100]);
    assert!(matches!(
        rig.actions.last(),
        Some(StepAction::Swing { from, .. }) if *from == [0, 40, 64, 104]
    ));
}

#[test]
fn an_undo_under_the_swing_popup_re_seeds_it_from_the_song() {
    let mut rig = Rig::new(sixteenths());
    let before = rig.channel().clone();
    rig.click(SWING);
    rig.drag_swing(0.5);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
    // An undo puts the hits and the lane's swing back while the popup is open: it shows 0 %, and 50 % applies again.
    rig.song.channels[CH] = before;
    rig.frame(vec![]);
    assert!(
        matches!(rig.view.overlay, Some(Overlay::Swing { amount, committed, .. })
            if amount == 0.0 && committed == 0.0),
        "{:?}",
        rig.view.overlay
    );
    rig.drag_swing(0.5);
    assert_eq!(rig.actions.len(), 2);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
    // An undo of an older step takes a hit away: the next release swings the hits that are left.
    rig.song.channels[CH].lanes[0].clips[0].events.pop();
    rig.frame(vec![]);
    rig.drag_swing(-0.5);
    assert_eq!(positions(&rig), [0, 32, 64]);
    assert_eq!(rig.channel().lanes[0].swing, 0.0);
}

#[test]
fn an_undo_of_a_swing_that_moved_no_hit_re_seeds_the_popup_too() {
    // Hits on the eighths only: swing moves none of them, so only the lane's swing tells the undo apart.
    let mut channel = drums();
    let c = channel.lanes[0].add_clip(0, 4.0);
    for step in [0, 2] {
        tap(c, 0, step);
    }
    let mut rig = Rig::new(channel);
    rig.click(SWING);
    rig.drag_swing(0.5);
    assert_eq!(
        (positions(&rig), rig.channel().lanes[0].swing),
        (vec![0, 64], 0.5)
    );
    rig.song.channels[CH].lanes[0].swing = 0.0;
    rig.frame(vec![]);
    assert!(
        matches!(rig.view.overlay, Some(Overlay::Swing { amount, committed, .. })
            if amount == 0.0 && committed == 0.0),
        "{:?}",
        rig.view.overlay
    );
    rig.drag_swing(0.5);
    assert_eq!(rig.actions.len(), 2);
    assert_eq!(rig.channel().lanes[0].swing, 0.5);
}

#[test]
fn the_swing_slider_opens_no_parameter_menu() {
    let mut rig = Rig::new(pattern());
    rig.click(SWING);
    let at = rig.area("swing").unwrap().center();
    rig.click_with(at, PointerButton::Secondary);
    assert!(matches!(rig.view.overlay, Some(Overlay::Swing { .. })));
    let texts: Vec<String> = rig.texts().into_iter().map(|t| t.0).collect();
    assert!(
        !texts.iter().any(|t| t == "Reset" || t == "Link to MIDI CC"),
        "{texts:?}"
    );
}

#[test]
fn the_swing_text_is_clamped() {
    for (swing, shown) in [(7.0, "100%"), (-1.0, "0%"), (f64::NAN, "0%")] {
        let mut channel = pattern();
        channel.lanes[0].swing = swing;
        let mut rig = Rig::new(channel);
        rig.click(SWING);
        let texts: Vec<String> = rig.texts().into_iter().map(|t| t.0).collect();
        assert!(texts.iter().any(|t| t == shown), "{swing}: {texts:?}");
    }
}

#[test]
fn a_non_finite_or_out_of_range_swing_seeds_the_popup_clamped() {
    for (swing, seed) in [(f64::NAN, 0.0), (f64::INFINITY, 1.0), (-3.0, 0.0)] {
        let mut channel = sixteenths();
        channel.lanes[0].swing = swing;
        let mut rig = Rig::new(channel);
        rig.click(SWING);
        assert!(
            matches!(rig.view.overlay, Some(Overlay::Swing { amount, committed, .. })
                if amount == seed && committed == seed),
            "{swing}: {:?}",
            rig.view.overlay
        );
        // At rest, nothing is applied, frame after frame.
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(rig.actions.is_empty(), "{swing}: {:?}", rig.actions);
    }
    // A drag from a NaN swing applies once, a clean amount.
    let mut channel = sixteenths();
    channel.lanes[0].swing = f64::NAN;
    let mut rig = Rig::new(channel);
    rig.click(SWING);
    rig.drag_swing(0.5);
    assert_eq!(rig.actions.len(), 1, "{:?}", rig.actions);
    assert_eq!(rig.channel().lanes[0].swing, 0.5);
    assert_eq!(positions(&rig), [0, 40, 64, 104]);
}

#[test]
fn the_swing_label_is_channel_coloured() {
    let rig = Rig::new(pattern());
    let ink = channel_color(rig.channel().hue, &theme_by_id(0)).to_color32();
    let swing = rig.texts().into_iter().find(|t| t.0 == "SWING").unwrap();
    assert_eq!(swing.1, ink);
}

#[test]
fn beat_groups_alternate_control_and_module_fill() {
    let rig = Rig::new(pattern());
    let colors = theme_by_id(0).colors;
    let (even, odd) = (
        rig.rects(colors.control_fill.to_color32()),
        rig.rects(colors.module_fill.to_color32()),
    );
    let l = rig.layout();
    for step in 0..16 {
        let (on, off) = if (step / 4) % 2 == 0 {
            (&even, &odd)
        } else {
            (&odd, &even)
        };
        assert!(on.contains(&l.cell(3, step)), "step {step}");
        assert!(!off.contains(&l.cell(3, step)), "step {step}");
    }
}

#[test]
fn a_lane_without_clips_takes_no_tap_or_fill_and_makes_no_clip() {
    // The playlist's `+Clip` makes clips; the steps edit the one the view is on.
    let mut rig = Rig::new(drums());
    let at = rig.cell(1, 2);
    rig.click(at);
    rig.menu(0, 1, 2);
    assert!(rig.channel().lanes[0].clips.is_empty());
}

#[test]
fn a_clip_id_that_is_gone_edits_nothing_and_makes_no_clip() {
    // The view follows its clip by id: it never moves to another clip of the lane.
    let mut rig = Rig::new(pattern());
    rig.view.clip = Some(5);
    rig.frame(vec![]);
    assert_eq!(rig.view.clip, Some(5));
    let at = rig.cell(3, 1);
    rig.click(at);
    assert!(!rig.hits().contains(&(3, 1, 0)));
    assert_eq!(rig.channel().lanes[0].clips.len(), 1);
}

#[test]
fn the_lock_button_toggles() {
    let mut rig = Rig::new(pattern());
    rig.click(LOCK);
    assert!(rig.view.lock);
    rig.frame(vec![]);
    rig.frame(vec![]);
    rig.click(LOCK);
    assert!(!rig.view.lock);
}

#[test]
fn rows_without_a_kit_pad_get_a_plain_name() {
    let mut channel = pattern();
    tap(&mut channel.lanes[0].clips[0], 5, 0);
    let rig = Rig::new(channel);
    assert_eq!(rig.layout().rows, 6);
    let texts: Vec<String> = rig
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    for name in ["Kick", "808 Kick", "Clap", "Pad 5", "Pad 6"] {
        assert!(texts.iter().any(|t| t == name), "{name}: {texts:?}");
    }
}

#[test]
fn a_lane_that_is_not_a_drum_lane_shows_nothing() {
    let mut channel = drums();
    channel.lanes[0].kind = LaneKind::Notes;
    let mut rig = Rig::new(channel);
    rig.click(pos2(300.0, 300.0));
    assert!(rig.actions.is_empty());
}

#[test]
fn row_headers_shrink_the_pad_name_and_truncate_the_sample_name() {
    // The drum mixer strip header: both lines left-aligned 2 hairlines into the header and at most its width less 7
    // hairlines; the pad name shrunk from 12 pt and centred on the row, the sample name truncated at 6 pt keeping 5
    // tail characters, 0.5 U lower.
    let sample = "A sample name wider than the header";
    let mut rig = Rig::new(drums());
    rig.kit.0[0] = ("Kick", sample, None);
    rig.frame(vec![]);
    let (h, t) = (rig.layout().header(0), metrics().t());
    let boxes = rig.header_lines(h);
    assert_eq!(boxes.len(), 2, "{boxes:?}");
    for b in &boxes {
        assert!(
            (b.1.left() - (h.left() + 2.0 * t)).abs() < 1e-3,
            "{b:?} in {h:?}"
        );
        assert!(b.1.width() <= h.width() - 7.0 * t, "{b:?}");
    }
    let (kick, cut) = (&boxes[0], &boxes[1]);
    assert_eq!((kick.0.as_str(), kick.2), ("Kick", 24.0));
    assert!((kick.1.center().y - h.center().y).abs() < 1e-3, "{kick:?}");
    let (head, tail) = cut.0.split_once("...").expect("an ellipsis");
    assert!(sample.starts_with(head) && !head.is_empty(), "{cut:?}");
    assert_eq!((tail, cut.2), ("eader", 12.0));
    assert!(
        (cut.1.center().y - (h.center().y + 0.5 * 32.0)).abs() < 1e-3,
        "{cut:?}"
    );
}

/// Twelve pads.
fn twelve() -> Kit {
    let names = [
        "Kick", "Snare", "Hat", "Clap", "Open Hat", "Tom", "Rim", "Crash", "Ride", "Shaker",
        "Cowbell", "Conga",
    ];
    Kit(names.map(|n| (n, "FL 808 Kit", None)).to_vec())
}

#[test]
fn a_new_clip_opens_at_2_u() {
    let mut rig = Rig::new(drums());
    assert_eq!(
        rig.row_view(),
        RowView {
            pitch: 2.0,
            scroll: 0.0
        }
    );
    // `+Clip`'s clip opens at 2 U too.
    rig.song.channels[CH].lanes[0].add_clip(0, 4.0);
    rig.view.clip = Some(1);
    rig.frame(vec![]);
    assert_eq!(
        rig.row_view(),
        RowView {
            pitch: 2.0,
            scroll: 0.0
        }
    );
    assert_eq!(
        rig.layout().header(3),
        Rect::from_min_max(pos2(0.0, 240.0), pos2(96.0, 303.0))
    );
}

#[test]
fn each_clip_keeps_its_own_zoom_and_scroll() {
    let mut channel = pattern();
    channel.lanes[0].add_clip(512, 4.0);
    let mut rig = Rig::new(channel);
    rig.kit = twelve();
    rig.pinch(pos2(400.0, 48.0), 1.5);
    rig.drag(pos2(400.0, 500.0), -100.0);
    let zoomed = rig.row_view();
    assert_eq!(zoomed.pitch, 3.0);
    assert!((zoomed.scroll - 100.0 / 32.0).abs() < 1e-3, "{zoomed:?}");
    rig.view.clip = Some(2);
    rig.frame(vec![]);
    assert_eq!(rig.row_view(), RowView::default(), "the other clip");
    rig.view.clip = Some(1);
    rig.frame(vec![]);
    assert_eq!(rig.row_view(), zoomed);
    // Another channel's view keeps them too: they are keyed by channel, lane and clip, not stored in the song.
    assert!(rig.view.views.contains_key(&(CH, 0, Some(1))));
}

#[test]
fn twelve_rows_scroll_to_the_last_row_and_the_add_button() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    // 12 rows of 64 and 96 of room in the 552-tall body: 312 to scroll. A drag on the grid scrolls, past the end at a
    // fifth (the rubber band), which springs back once let go.
    rig.drag(pos2(400.0, 590.0), -400.0);
    assert!(rig.row_view().scroll > 312.0 / 32.0);
    for _ in 0..80 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.row_view().scroll, 312.0 / 32.0);
    let l = rig.layout();
    let body = l.body();
    // The header column scrolls with the grid.
    assert_eq!(l.header(0).top(), 48.0 - 312.0);
    assert_eq!(
        l.header(11),
        Rect::from_min_max(pos2(0.0, 440.0), pos2(96.0, 503.0))
    );
    assert_eq!(l.cell(11, 15).y_range(), l.header(11).y_range());
    let add = l.add_button();
    assert_eq!(
        add,
        Rect::from_min_size(pos2(16.0, 528.0), vec2(64.0, 64.0))
    );
    assert!(body.contains_rect(add));
    // The `+` is drawn there.
    assert!(
        rig.shapes.iter().any(|c| {
            let r = c.shape.visual_bounding_rect();
            r.is_positive() && add.contains_rect(r) && c.clip_rect.contains_rect(r)
        }),
        "no + drawn in {add:?}"
    );
    // The last row is live: its header selects, its steps toggle.
    rig.click(l.header(11).center());
    assert_eq!(rig.view.selected, 11);
    rig.click(l.cell(11, 15).center());
    assert!(rig.hits().contains(&(11, 15, 0)), "{:?}", rig.hits());
    // Nothing paints or reacts over the `SWING` strip: row 0's header there is clipped away.
    rig.click(pos2(48.0, 40.0));
    assert!(matches!(rig.view.overlay, Some(Overlay::Swing { .. })));
}

#[test]
fn headers_span_the_pitch_and_their_lines_sit_on_the_middle_and_half_a_u_below() {
    let t = metrics().t();
    for pitch in [2.0, 4.0] {
        let mut rig = Rig::new(pattern());
        rig.set_row_view(RowView { pitch, scroll: 0.0 });
        let p = pitch * 32.0;
        for row in 0..4 {
            let h = rig.layout().header(row);
            let top = 48.0 + row as f32 * p;
            assert_eq!(
                h,
                Rect::from_min_max(pos2(0.0, top), pos2(96.0, top + p - 1.0)),
                "{pitch} U, row {row}"
            );
            let lines = rig.header_lines(h);
            assert_eq!(lines.len(), 2, "{lines:?}");
            let (pad, sample) = (&lines[0], &lines[1]);
            assert_eq!(
                (pad.0.as_str(), sample.0.as_str()),
                (kit().0[row].0, kit().0[row].1)
            );
            for line in &lines {
                assert!((line.1.left() - 2.0 * t).abs() < 1e-3, "{line:?}");
            }
            assert!(
                (pad.1.center().y - h.center().y).abs() < 1e-3,
                "{pad:?} in {h:?}"
            );
            assert!(
                (sample.1.center().y - (h.center().y + 16.0)).abs() < 1e-3,
                "{sample:?} in {h:?}"
            );
        }
    }
}

#[test]
fn no_header_line_is_clipped_at_2_u() {
    // With the app's font, twelve rows: at 2 U both lines of every header in view fit inside it.
    let mut rig = Rig::new(pattern());
    crate::fonts::install(&rig.ctx);
    rig.kit = twelve();
    rig.frame(vec![]);
    rig.frame(vec![]);
    let l = rig.layout();
    let mut lines = 0;
    for row in 0..12 {
        let h = l.header(row);
        if !l.body().contains_rect(h) {
            continue;
        }
        for (text, r, _, clip) in rig.header_lines(h) {
            assert!(
                clip.contains_rect(r) && h.contains_rect(r),
                "{text}: {r:?} in {h:?}, clip {clip:?}"
            );
            lines += 1;
        }
    }
    // Rows 0 to 7 are wholly in view.
    assert_eq!(lines, 16);
}

#[test]
fn a_trackpad_pinch_zooms_the_time_and_two_fingers_zoom_the_rows() {
    // C rounds 17, 21, 25: the trackpad pinch over the grid is the time's (a beat 1.5 times as wide, k 0.33 → 0.22);
    // the rows stay. Two fingers zoom the rows.
    let mut rig = Rig::new(pattern());
    let before = rig.row_view();
    rig.frame(vec![egui::Event::PointerMoved(pos2(400.0, 200.0))]);
    rig.frame(vec![egui::Event::Zoom(1.5)]);
    rig.frame(vec![]);
    assert_eq!(rig.row_view(), before);
    assert!(
        (rig.clip_view().k - 0.22).abs() < 1e-5,
        "{:?}",
        rig.clip_view()
    );
    rig.pinch(pos2(400.0, 48.0), 1.5);
    assert_eq!(rig.row_view().pitch, before.pitch * 1.5);
}

#[test]
fn a_pinch_keeps_the_point_under_the_pointer() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    // 100 points into the rows, 1.5625 rows down; at 3 U the same row point is 150 points down, so 50 scroll.
    let at = pos2(400.0, 148.0);
    let row_at = |rig: &Rig| {
        let l = rig.layout();
        (at.y - l.row_top()) / l.pitch
    };
    assert_eq!(row_at(&rig), 1.5625);
    rig.pinch(at, 1.5);
    let view = rig.row_view();
    assert!((view.scroll - 50.0 / 32.0).abs() < 1e-5, "{view:?}");
    assert_eq!(
        rig.row_view(),
        RowView {
            pitch: 3.0,
            scroll: view.scroll
        }
    );
    assert_eq!(row_at(&rig), 1.5625);
}

#[test]
fn the_wheel_scrolls_the_rows() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![egui::Event::PointerMoved(pos2(400.0, 300.0))]);
    rig.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -100.0),
        modifiers: Modifiers::NONE,
    }]);
    // egui smooths the wheel over a few frames.
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    let scroll = rig.row_view().scroll;
    assert!((scroll - 100.0 / 32.0).abs() < 0.01, "{scroll}");
}

#[test]
fn each_lane_keeps_its_own_clip_views() {
    // Clip ids are unique only within a lane: the first clip of either lane is clip 1.
    let mut channel = pattern();
    let mut second = Lane::new(LaneKind::Drums);
    second.add_clip(0, 4.0);
    channel.lanes.push(second);
    assert_eq!(channel.lanes[1].clips[0].id, channel.lanes[0].clips[0].id);
    let mut rig = Rig::new(channel);
    rig.kit = twelve();
    rig.pinch(pos2(400.0, 48.0), 1.5);
    assert_eq!(rig.row_view().pitch, 3.0);
    rig.view.lane = 1;
    rig.frame(vec![]);
    assert_eq!(rig.row_view(), RowView::default(), "lane 1's clip 1");
    rig.view.lane = 0;
    rig.frame(vec![]);
    assert_eq!(rig.row_view().pitch, 3.0);
}

// The step selection and its floating buttons.

#[test]
fn copy_steps_puts_the_copies_right_after_the_selection_over_what_was_there() {
    let mut c = clip(4.0);
    // Kick a little late in step 0 (swung), Hat on 1, Snare on 2; a default Snare hit on 5.
    c.events = vec![drum(0, 5), drum(2, STEP_TICKS), drum(1, 2 * STEP_TICKS)];
    tap(&mut c, 1, 5);
    let copies = copy_steps(&mut c, &[(0, 0), (1, 2)], SIG);
    // Steps 0 to 2 are three steps: the copies land three steps on, the Hat (not selected) stays where it is.
    assert_eq!(copies, [(0, 3), (1, 5)]);
    assert_eq!(
        cells(&c),
        [(0, 0, 5), (2, 1, 0), (1, 2, 0), (0, 3, 5), (1, 5, 0)]
    );
    // The copy took the place of the hit on 5, and keeps everything of its own hit.
    assert_eq!(c.events[4], drum(1, 5 * STEP_TICKS));
    assert_eq!(c.loop_len_beats, 4.0);
}

#[test]
fn copy_steps_past_the_loop_end_puts_the_copies_there() {
    // The loop is the host's to fit after it, as after every hit edit (see the test below).
    let mut c = clip(4.0);
    tap(&mut c, 0, 12);
    tap(&mut c, 0, 15);
    assert_eq!(
        copy_steps(&mut c, &[(0, 12), (0, 15)], SIG),
        [(0, 16), (0, 19)]
    );
    assert_eq!(cells(&c), on(&[(0, 12), (0, 15), (0, 16), (0, 19)]));
    assert_eq!(c.loop_len_beats, 4.0);
}

#[test]
fn every_hit_edit_fits_the_loop_to_the_smallest_bar_power_of_two_which_may_shrink_it() {
    // The loop is recomputed on every clip update, `Clip::fit_loop`, as the free grid's and the piano roll's. The clip
    // is a new one (visible length 0), so its length, the second number, follows the loop; a sized clip: the next test.
    let fitted = |beats: f64, hits: &[(u8, usize)], action: StepAction| -> (f64, f64) {
        let mut channel = drums();
        let c = channel.lanes[0].add_clip(0, beats);
        for &(pad, step) in hits {
            tap(c, pad, step);
        }
        // On the clip, as the app opens it.
        let mut view = StepSeqView {
            clip: Some(1),
            ..StepSeqView::default()
        };
        view.apply_to(&mut channel, SIG, &action);
        let c = &channel.lanes[0].clips[0];
        assert_eq!(c.visible_len_beats, 0.0);
        (
            c.loop_len_beats,
            c.visible_ticks() / f64::from(crate::song::PPQ),
        )
    };
    use StepAction::{Clear, CopySteps, DeleteSteps, Fill, Rotate, Toggle};
    // Two bars whose second one's only hit goes: one bar.
    let two = [(0, 0), (1, 20)];
    assert_eq!(fitted(8.0, &two, Toggle { pad: 1, step: 20 }), (4.0, 4.0));
    assert_eq!(fitted(8.0, &two, Clear { pad: 1 }), (4.0, 4.0));
    let cells = vec![(1, 20)];
    assert_eq!(fitted(8.0, &two, DeleteSteps { cells }), (4.0, 4.0));
    // The last step's hit rotates right onto step 0.
    let rotate = Rotate {
        pad: 1,
        right: true,
    };
    assert_eq!(fitted(8.0, &[(0, 0), (1, 31)], rotate), (4.0, 4.0));
    // A copy inside bar 1 of two bars: one.
    let cells = vec![(0, 0)];
    assert_eq!(fitted(8.0, &[(0, 0)], CopySteps { cells }), (4.0, 4.0));
    // A tap, copies or Fill reaching bar 3: four bars, not three.
    assert_eq!(fitted(12.0, &[], Toggle { pad: 0, step: 40 }), (16.0, 16.0));
    let cells = vec![(0, 4), (0, 20)];
    assert_eq!(
        fitted(8.0, &[(0, 4), (0, 20)], CopySteps { cells }),
        (16.0, 16.0)
    );
    assert_eq!(fitted(12.0, &[], Fill { pad: 0, every: 1 }), (16.0, 16.0));
    // The row menu's `Delete` of the second bar's only pad: one bar.
    assert_eq!(fitted(8.0, &two, StepAction::Delete { pad: 1 }), (4.0, 4.0));
    // Swing: the last step's hit swung past bar 2 makes four bars; on an even step nothing moves, but the loop fits.
    let swing = |from: u32| StepAction::Swing {
        amount: 0.5,
        from: vec![from],
        final_edit: true,
    };
    let last = 31 * crate::song::PPQ / 4;
    assert_eq!(fitted(8.0, &[(0, 31)], swing(last)), (16.0, 16.0));
    assert_eq!(fitted(8.0, &[(0, 0)], swing(0)), (4.0, 4.0));
}

#[test]
fn a_sized_clip_keeps_its_length_when_a_hit_edit_refits_the_loop() {
    // Once the clip has been resized in the playlist its length stays fixed and the content loops inside it, a length
    // equal to the loop included.
    let fitted = |loop_beats: f64, sized: f64, hits: &[(u8, usize)], action: StepAction| {
        let mut channel = drums();
        let c = channel.lanes[0].add_clip(0, loop_beats);
        c.visible_len_beats = sized;
        for &(pad, step) in hits {
            tap(c, pad, step);
        }
        // On the clip, as the app opens it.
        let mut view = StepSeqView {
            clip: Some(1),
            ..StepSeqView::default()
        };
        view.apply_to(&mut channel, SIG, &action);
        let c = &channel.lanes[0].clips[0];
        (c.loop_len_beats, c.visible_len_beats)
    };
    // A tap at step 40 (beat 10) doubles one bar to four; the 4 beats shown stay 4.
    let tap40 = StepAction::Toggle { pad: 0, step: 40 };
    assert_eq!(fitted(4.0, 4.0, &[], tap40), (16.0, 4.0));
    // The last bar's only hit goes: the loop is one bar again, the 8 beats shown stay 8 and repeat it.
    let clear = StepAction::Clear { pad: 1 };
    assert_eq!(fitted(8.0, 8.0, &[(0, 0), (1, 20)], clear), (4.0, 8.0));
}

#[test]
fn paste_steps_lands_the_first_copied_step_on_the_target_keeping_pads_and_offsets() {
    // At d = 3 steps start at ticks 0, 43, 85, 128: hits on steps 1 and 2 (the second 5 ticks in) pasted at step 0.
    let three = TimeSig {
        subdivision: 3,
        ..SIG
    };
    let hits = [drum(0, 43), drum(1, 90)];
    let mut c = clip(4.0);
    c.events = vec![drum(1, 50), drum(2, 50)];
    assert_eq!(paste_steps(&mut c, &hits, 0, three), [(0, 0), (1, 1)]);
    // The paste took the place of pad 1's hit on step 1; pad 2's stays.
    let pads_pos: Vec<_> = (c.events.iter())
        .map(|e| (cell_of(e, three).unwrap().0, e.pos()))
        .collect();
    assert_eq!(pads_pos, [(0, 0), (1, 48), (2, 50)]);
    // Past the grid's 512 steps: dropped.
    let mut c = clip(4.0);
    let last = MAX_STEPS - 1;
    assert_eq!(paste_steps(&mut c, &hits, last, three), [(0, last)]);
    assert!(paste_steps(&mut c, &[], 0, three).is_empty());
}

#[test]
fn copy_steps_drops_the_copies_past_the_512_step_cap() {
    let mut c = clip(128.0);
    tap(&mut c, 0, 500);
    tap(&mut c, 0, 510);
    assert_eq!(copy_steps(&mut c, &[(0, 500), (0, 510)], SIG), [(0, 511)]);
    assert_eq!(cells(&c), on(&[(0, 500), (0, 510), (0, 511)]));
    assert_eq!(c.loop_len_beats, 128.0);
}

#[test]
fn delete_steps_removes_every_hit_of_the_selected_cells_only() {
    let mut c = clip(4.0);
    for (pad, step) in [(0, 0), (2, 2), (1, 4), (1, 12)] {
        tap(&mut c, pad, step);
    }
    // A second, late hit in the Snare's step 4.
    c.events.insert(3, drum(1, 4 * STEP_TICKS + 7));
    delete_steps(&mut c, &[(1, 4), (0, 0)], SIG);
    assert_eq!(cells(&c), on(&[(2, 2), (1, 12)]));
}

impl Rig {
    /// A frame `dt` seconds after the last.
    fn frame_after(&mut self, dt: f64, events: Vec<egui::Event>) {
        self.time += dt - 0.1;
        self.frame(events);
    }

    /// The primary button pressed at `pos` and held there for a second: a long press.
    fn hold(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.press(pos, PointerButton::Primary, true);
        self.frame_after(1.0, vec![]);
    }

    fn release(&mut self, pos: Pos2) {
        self.press(pos, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    /// A finger: the touch event plus the pointer events an integration sends with it, `dt` after the last frame.
    fn touch(&mut self, dt: f64, phase: egui::TouchPhase, pos: Pos2) {
        let mut events = vec![egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        }];
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        events.extend(match phase {
            egui::TouchPhase::Start => vec![egui::Event::PointerMoved(pos), button(true)],
            egui::TouchPhase::Move => vec![egui::Event::PointerMoved(pos)],
            _ => vec![button(false), egui::Event::PointerGone],
        });
        self.frame_after(dt, events);
    }

    /// Centre of the text `label` painted in the last frame.
    fn text_at(&self, label: &str) -> Option<Pos2> {
        self.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
    }

    fn selection(&self) -> Vec<(u8, usize)> {
        self.view.selection.iter().copied().collect()
    }
}

#[test]
fn a_long_press_on_a_hit_selects_it_and_floats_copy_and_delete_over_it() {
    let mut rig = Rig::new(pattern());
    assert_eq!(rig.text_at("Copy"), None);
    let at = rig.cell(1, 4);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selection(), [(1, 4)]);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // Step-sequencer mode: no `Snap`. The row sits centred above the selected step.
    let (copy, delete) = (rig.text_at("Copy").unwrap(), rig.text_at("Delete").unwrap());
    assert_eq!(rig.text_at("Snap"), None);
    let cell = rig.layout().cell(1, 4);
    assert!(copy.y < cell.top() && delete.y < cell.top());
    assert!(((copy.x + delete.x) / 2.0 - cell.center().x).abs() < 1.0);
}

#[test]
fn a_selected_hit_takes_the_selected_tint_and_the_others_the_tint() {
    let mut rig = Rig::new(pattern());
    let at = rig.cell(1, 4);
    rig.hold(at);
    rig.release(at);
    let ink = tinted(&rig, Prop::Velocity, 255, false);
    let pale = tinted(&rig, Prop::Velocity, 255, true);
    let l = rig.layout();
    let in_grid = |color| -> Vec<Rect> {
        let rects = rig.rects(color).into_iter();
        rects.filter(|r| l.grid().contains_rect(*r)).collect()
    };
    let pale = in_grid(pale);
    assert_eq!(pale.len(), 1);
    assert!(l.cell(1, 4).contains_rect(pale[0]));
    assert_eq!(in_grid(ink).len(), 3);
}

#[test]
fn a_long_press_then_a_drag_boxes_the_hits_without_scrolling() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    // Up and left: a drag up would scroll the rows down.
    let (from, to) = (rig.cell(2, 4), rig.cell(0, 1));
    rig.hold(from);
    rig.frame(vec![egui::Event::PointerMoved(from + (to - from) / 2.0)]);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    // The box shows while it is dragged; the buttons wait for it.
    assert_eq!(rig.text_at("Copy"), None);
    rig.release(to);
    // The Kick on step 0 is left of the box.
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    assert_eq!(rig.row_view().scroll, 0.0);
    assert!(rig.actions.is_empty());
    // A long press on empty space, lifted where it was, selects nothing.
    let at = rig.cell(3, 3);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty());
}

#[test]
fn a_stationary_hold_on_an_empty_step_has_no_marquee_until_dragged() {
    // The marquee, with the mouse held 0.35 s.
    let mut rig = Rig::new(pattern());
    let from = rig.cell(1, 1);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    rig.frame_after(0.35, vec![]);
    let square = |rig: &Rig, size: f32| {
        (rig.shapes.iter()).any(|s| match &s.shape {
            egui::Shape::Rect(r) => {
                (r.rect.center() - from).length() < 0.5 && (r.rect.width() - size).abs() < 0.5
            }
            _ => false,
        })
    };
    assert!(!square(&rig, 96.0));
    assert_eq!(rig.selection(), []);
    let to = rig.cell(2, 5);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    rig.release(to);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    assert!(rig.actions.is_empty());
}

#[test]
fn a_press_held_over_a_focus_loss_is_up_and_starts_no_box_and_no_tap() {
    // The grid read the raw button, which egui keeps down over a focus loss (the release may never come): the press
    // went on, a box started at the long press and followed the hover. As in the other editors (`primary_down_live`),
    // it is up: no tap, no box; a box under way ends where it was.
    let mut rig = Rig::new(pattern());
    let from = rig.cell(1, 1);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    rig.frame_after(0.5, vec![egui::Event::WindowFocused(true)]);
    rig.frame(vec![egui::Event::PointerMoved(rig.cell(2, 5))]);
    assert_eq!(rig.view.boxing, None);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    rig.release(rig.cell(2, 5));
    // Held 0.5 s with no frame, then the focus goes as the pointer moves far: its position is no drag.
    let mut rig = Rig::new(pattern());
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    let far = rig.cell(3, 9);
    rig.frame_after(
        0.5,
        vec![
            egui::Event::WindowFocused(false),
            egui::Event::PointerMoved(far),
        ],
    );
    assert_eq!((rig.view.boxing, rig.selection()), (None, vec![]));
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // A box, then the focus goes: it ends with what it held.
    let mut rig = Rig::new(pattern());
    rig.hold(from);
    rig.frame(vec![egui::Event::PointerMoved(rig.cell(2, 5))]);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    rig.frame(vec![
        egui::Event::WindowFocused(true),
        egui::Event::PointerMoved(rig.cell(3, 15)),
    ]);
    assert_eq!(rig.view.boxing, None);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
}

#[test]
fn the_step_views_wheel_zooms_the_rows_on_the_headers_and_scrolls_them_on_the_grid() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    let wheel = |rig: &mut Rig, at: Pos2, delta: egui::Vec2| {
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers: Modifiers::NONE,
            },
        ]);
    };
    let header = rig.cell(2, 0) - vec2(60.0, 0.0);
    wheel(&mut rig, header, vec2(0.0, 200.0));
    assert!(
        (rig.row_view().pitch - 2.5).abs() < 1e-5,
        "{:?}",
        rig.row_view()
    );
    let (pitch, scroll) = (rig.row_view().pitch, rig.row_view().scroll);
    let cell = rig.cell(2, 4);
    wheel(&mut rig, cell, vec2(0.0, -64.0));
    assert!(
        (rig.row_view().scroll - scroll - 2.0).abs() < 1e-5,
        "{:?}",
        rig.row_view()
    );
    assert_eq!(rig.row_view().pitch, pitch);
    // The ruler: the time zooms (the step grid has a time axis), the rows stay.
    let before = rig.row_view();
    wheel(&mut rig, pos2(400.0, 20.0), vec2(0.0, 200.0));
    assert_eq!(rig.row_view(), before);
    assert!((rig.clip_view().k - 0.33 * 0.8).abs() < 1e-5);
}

#[test]
fn a_touch_long_press_selects_and_a_move_boxes() {
    use egui::TouchPhase::*;
    let mut rig = Rig::new(pattern());
    let at = rig.cell(2, 2);
    rig.touch(0.1, Start, at);
    rig.touch(1.0, Move, at);
    rig.touch(0.1, End, at);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), [(2, 2)]);
    let (from, to) = (rig.cell(0, 0), rig.cell(1, 12));
    rig.touch(0.1, Start, from);
    rig.touch(1.0, Move, from);
    rig.touch(0.1, Move, to);
    rig.touch(0.1, End, to);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), [(0, 0), (1, 4), (1, 12)]);
    assert!(rig.actions.is_empty());
}

impl Rig {
    /// A long press from `from` dragged to `to`: the box over the cells between.
    fn boxed(&mut self, from: Pos2, to: Pos2) {
        self.hold(from);
        self.frame(vec![egui::Event::PointerMoved(to)]);
        self.release(to);
    }
}

#[test]
fn a_tap_on_a_hit_deletes_it_when_it_alone_is_selected_else_only_deselects() {
    // In step mode the touch-down makes the hit the selection, so a tap deletes it ...
    let mut rig = Rig::new(pattern());
    let snare = rig.cell(1, 4);
    rig.hold(snare);
    rig.release(snare);
    assert_eq!(rig.selection(), [(1, 4)]);
    // ... even with another hit selected (outside the selection's frame it becomes the selection).
    rig.click(rig.cell(0, 0));
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 0, step: 0 }]);
    assert_eq!(rig.selection(), []);
    // Inside a frame of two, a tap only deselects.
    rig.actions.clear();
    rig.boxed(rig.cell(1, 1), rig.cell(2, 5));
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    rig.click(snare);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // A tap on an empty step while a selection shows only deselects; then it toggles.
    rig.boxed(rig.cell(1, 1), rig.cell(2, 5));
    let empty = rig.cell(3, 7);
    rig.click(empty);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty());
    rig.click(empty);
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 3, step: 7 }]);
}

#[test]
fn a_long_press_on_a_selected_hit_keeps_the_selection() {
    let mut rig = Rig::new(pattern());
    rig.boxed(rig.cell(1, 1), rig.cell(2, 5));
    let snare = rig.cell(1, 4);
    rig.hold(snare);
    rig.release(snare);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    assert!(rig.text_at("Copy").is_some());
    assert!(rig.actions.is_empty());
}

#[test]
fn a_right_click_makes_the_hit_under_it_the_selection_and_on_an_empty_step_changes_nothing() {
    let mut rig = Rig::new(pattern());
    let at = rig.cell(2, 2);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.selection(), [(2, 2)]);
    let at = rig.cell(1, 12);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.selection(), [(1, 12)]);
    // An empty step: `Paste`, and there is no step clipboard.
    let at = rig.cell(0, 9);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.selection(), [(1, 12)]);
    assert!(rig.actions.is_empty());
}

#[test]
fn copy_duplicates_the_selected_steps_to_the_right_and_selects_the_copies() {
    let mut rig = Rig::new(pattern());
    let (from, to) = (rig.cell(0, 0), rig.cell(2, 4));
    rig.hold(from);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    rig.release(to);
    let copy = rig.text_at("Copy").unwrap();
    rig.click(copy);
    let cells = vec![(0, 0), (1, 4), (2, 2)];
    assert_eq!(rig.actions, [StepAction::CopySteps { cells }]);
    // Steps 0 to 4 are five steps.
    assert_eq!(
        rig.hits(),
        on(&[(0, 0), (2, 2), (1, 4), (0, 5), (2, 7), (1, 9), (1, 12)])
    );
    assert_eq!(rig.selection(), [(0, 5), (1, 9), (2, 7)]);
}

#[test]
fn copy_fills_the_hit_clipboard_and_a_long_press_or_right_click_on_an_empty_step_pastes_it() {
    // As in the automation editor, `Copy` also stores the hits; lifted in place on an empty step, a long press shows
    // `Paste` while the clipboard is set, and its tap lands the first copied step on the step under it, the others
    // keeping their pads and distances, selected.
    let mut rig = Rig::new(pattern());
    let empty = rig.cell(3, 8);
    rig.hold(empty);
    rig.release(empty);
    assert_eq!(rig.text_at("Paste"), None, "nothing to paste");
    rig.click_with(empty, PointerButton::Secondary);
    assert_eq!(rig.text_at("Paste"), None);
    rig.boxed(rig.cell(0, 0), rig.cell(2, 4));
    rig.click(rig.text_at("Copy").unwrap());
    let copied: Vec<_> = (rig.channel().lanes[0].clips[0].events.iter())
        .filter(|e| [0, 2 * STEP_TICKS, 4 * STEP_TICKS].contains(&e.pos()))
        .copied()
        .collect();
    assert_eq!(hit_clipboard(&rig.ctx), copied);
    rig.hold(empty);
    assert!(
        rig.text_at("Paste").is_some(),
        "Paste appears before lifting"
    );
    rig.release(empty);
    let paste = rig.text_at("Paste").expect("the Paste button");
    assert!(paste.distance(empty) < 2.0);
    rig.click(paste);
    let hits = copied.clone();
    assert_eq!(
        rig.actions.last(),
        Some(&StepAction::PasteSteps { step: 8, hits })
    );
    // Over the Snare on 12.
    assert_eq!(
        rig.hits(),
        on(&[
            (0, 0),
            (2, 2),
            (1, 4),
            (0, 5),
            (2, 7),
            (0, 8),
            (1, 9),
            (2, 10),
            (1, 12)
        ])
    );
    assert_eq!(rig.selection(), [(0, 8), (1, 12), (2, 10)]);
    assert_eq!(rig.text_at("Paste"), None);
    // A right click on an empty step shows it too, and a press anywhere else hides it.
    let empty = rig.cell(3, 14);
    rig.click_with(empty, PointerButton::Secondary);
    assert!(rig.text_at("Paste").is_some());
    rig.click(rig.cell(3, 1));
    assert_eq!(rig.text_at("Paste"), None);
}

#[test]
fn delete_clears_the_selected_steps_and_the_selection() {
    let mut rig = Rig::new(pattern());
    rig.boxed(rig.cell(0, 0), rig.cell(1, 4));
    let delete = rig.text_at("Delete").unwrap();
    rig.click(delete);
    let cells = vec![(0, 0), (1, 4)];
    assert_eq!(rig.actions, [StepAction::DeleteSteps { cells }]);
    assert_eq!(rig.hits(), on(&[(2, 2), (1, 12)]));
    assert_eq!(rig.selection(), []);
    assert_eq!(rig.text_at("Delete"), None);
}

#[test]
fn a_selected_step_that_loses_its_hit_leaves_the_selection() {
    // An undo, a row's Clear or Rotate: the song changes under the selection.
    let mut rig = Rig::new(pattern());
    rig.boxed(rig.cell(1, 1), rig.cell(2, 5));
    rig.song.channels[CH].lanes[0].clips[0]
        .events
        .retain(|e| e.pos() != 2 * STEP_TICKS);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), [(1, 4)]);
}

#[test]
fn the_selection_stays_with_its_clip() {
    // Clip B of the same lane has the same Snare hit: a selection kept across would show on it and delete it.
    let mut channel = pattern();
    let b = channel.lanes[0].clips[0].clone();
    channel.lanes[0].add_clip(16 * crate::song::PPQ, 4.0).events = b.events;
    let mut rig = Rig::new(channel);
    let at = rig.cell(1, 4);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.selection(), [(1, 4)]);
    rig.view.clip = Some(2);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), []);
    assert_eq!(rig.text_at("Delete"), None);
    // Back on A, it is gone too: a selection belongs to one visit of one clip.
    rig.view.clip = Some(1);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), []);
}

#[test]
fn the_floating_row_hides_on_a_scroll_or_zoom_and_a_tap_on_the_selection_brings_it_back() {
    // The same rule as the playlist and the piano roll: the shared placement, hidden by any scroll or zoom, back after
    // a release on the selection. At U = 32: a 2-button row is 102.4 × 48, m 6.4.
    let shared = |rig: &Rig, cell: (usize, usize), finger: Pos2| {
        let l = rig.layout();
        let sel = Selection {
            rect: l.cell(cell.0, cell.1),
            one_row: true,
            handles: &[],
        };
        place_row(&sel, finger, l.grid(), vec2(102.4, 48.0), 6.4, 1.0)
    };
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    let at = rig.cell(1, 4);
    rig.click_with(at, PointerButton::Secondary);
    assert!(rig.text_at("Copy").is_some());
    assert_eq!(rig.view.row.rect(), shared(&rig, (1, 4), at));
    // The wheel scrolls the rows: hidden, though the hit is still in view.
    rig.frame(vec![egui::Event::PointerMoved(pos2(400.0, 300.0))]);
    rig.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -20.0),
        modifiers: Modifiers::NONE,
    }]);
    for _ in 0..30 {
        rig.frame(vec![]);
    }
    assert!(rig.row_view().scroll > 0.0);
    assert_eq!(rig.text_at("Copy"), None);
    assert_eq!(rig.selection(), [(1, 4)]);
    // A long press on the selected hit shows it at the new place, the hit still selected (a tap would delete it).
    let at = rig.cell(1, 4);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selection(), [(1, 4)]);
    assert!(rig.text_at("Copy").is_some());
    assert_eq!(rig.view.row.rect(), shared(&rig, (1, 4), at));
    // A pinch zooms: hidden again.
    rig.pinch(pos2(400.0, 300.0), 1.5);
    assert_eq!(rig.text_at("Copy"), None);
    assert_eq!(rig.selection(), [(1, 4)]);
}

#[test]
fn the_floating_row_of_a_box_selection_shows_at_its_release() {
    // The release (step 5) lies off the selection's bounds (steps 2 to 4): the box made the selection.
    let mut rig = Rig::new(pattern());
    let (from, to) = (rig.cell(1, 0), rig.cell(2, 5));
    rig.hold(from);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    rig.release(to);
    assert_eq!(rig.selection(), [(1, 4), (2, 2)]);
    let l = rig.layout();
    let sel = Selection {
        rect: l.cell(1, 4).union(l.cell(2, 2)),
        one_row: false,
        handles: &[],
    };
    let want = place_row(&sel, to, l.grid(), vec2(102.4, 48.0), 6.4, 1.0);
    assert!(want.is_some());
    assert_eq!(rig.view.row.rect(), want);
}

#[test]
fn a_rows_clone_or_delete_clears_the_selection() {
    // The Hat gets a hit on 4 too, so the shifted rows still hold a hit in the selected cell.
    let cases = [
        (StepAction::Clone { pad: 1 }, (2, 4)),
        (StepAction::Delete { pad: 1 }, (1, 4)),
    ];
    for (action, (row, step)) in cases {
        let mut channel = pattern();
        tap(&mut channel.lanes[0].clips[0], 2, 4);
        let mut rig = Rig::new(channel);
        let at = rig.cell(row, step);
        rig.click_with(at, PointerButton::Secondary);
        rig.view.apply(&mut rig.song, CH, &action);
        rig.frame(vec![]);
        assert_eq!(rig.selection(), [], "{action:?}");
    }
}

#[test]
fn a_second_finger_cancels_the_box_and_starts_none() {
    use egui::TouchPhase::*;
    let finger = |id: u64, phase, pos| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(id),
        phase,
        pos,
        force: None,
    };
    let mut rig = Rig::new(pattern());
    // On an empty step, where a long press starts a box.
    let (a, b) = (rig.cell(0, 1), rig.cell(2, 2));
    rig.touch(0.1, Start, a);
    rig.touch(1.0, Move, a);
    assert!(rig.view.boxing.is_some());
    // A second finger: a pinch, not a box.
    rig.frame(vec![finger(2, Start, b)]);
    assert_eq!(rig.view.boxing, None);
    // Held still, the two fingers start no box either.
    rig.frame_after(1.0, vec![finger(2, Move, b)]);
    assert_eq!(rig.view.boxing, None);
    rig.frame(vec![finger(2, End, b)]);
    rig.touch(0.1, End, a);
    rig.frame(vec![]);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty());
}

// The graph lane.

/// The knob on the view's bottom edge while the lane is hidden (x 1.4 U, its upper half in the view), and the lane
/// at its first show (f 0.5 of H = 600, the view below its top window margin, which is 0: the `SWING` strip counts):
/// its strip from y 300, its list and value area under the 4 px gap, 296 tall, the centre band around y 452.
const KNOB: Pos2 = pos2(44.8, 595.0);
const STRIP_TOP: f32 = 300.0;
const LANE_TOP: f32 = 304.0;
const LANE_MID: f32 = 452.0;

/// The y `v` of the way up the value area.
fn lane_y(v: f32) -> f32 {
    600.0 - v * (600.0 - LANE_TOP)
}

impl Rig {
    /// The x of `step`'s centre.
    fn step_x(&self, step: usize) -> f32 {
        self.cell(0, step).x
    }

    /// Shows the lane with a tap on its knob.
    fn open_lane(&mut self) {
        self.click(KNOB);
        assert!(self.view.graph.open);
    }

    /// Presses at the first point, moves to each of the others (one frame each) and releases at the last.
    fn stroke(&mut self, points: &[Pos2]) {
        self.frame(vec![egui::Event::PointerMoved(points[0])]);
        self.press(points[0], PointerButton::Primary, true);
        for &p in &points[1..] {
            self.frame(vec![egui::Event::PointerMoved(p)]);
        }
        let last = points[points.len() - 1];
        self.press(last, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    /// (pad, velocity, pan, pitch, repeat) of each hit, in event order.
    fn values(&self) -> Vec<(u8, u8, u8, u16, u8)> {
        self.channel().lanes[0].clips[0]
            .events
            .iter()
            .map(|e| match *e {
                Event::Drum {
                    pad,
                    velocity,
                    pan,
                    value,
                    repeat,
                    ..
                } => (pad, velocity, pan, value, repeat),
                _ => panic!("not a drum hit"),
            })
            .collect()
    }
}

/// A paint sample on the Snare row (pad 1).
fn paint(prop: Prop, step: usize, value: u16) -> StepAction {
    StepAction::Graph {
        pad: 1,
        prop,
        step,
        value,
    }
}

#[test]
fn the_lane_is_hidden_until_its_knob_is_tapped_and_lists_its_four_items_in_the_header_column() {
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    rig.frame(vec![]);
    assert_eq!(rig.text_at("Velocity"), None);
    // Hidden, the rows reach the bottom: row 8 (y 560..623) is live there.
    let at = rig.cell(8, 3);
    rig.click(at);
    assert!(rig.hits().contains(&(8, 3, 0)), "{:?}", rig.hits());
    rig.open_lane();
    let at: Vec<Pos2> = ["Velocity", "Panning", "Pitch", "Repeat"]
        .map(|l| rig.text_at(l).unwrap_or_else(|| panic!("{l}")))
        .to_vec();
    for w in at.windows(2) {
        assert!(w[0].y < w[1].y, "{at:?}");
    }
    for p in &at {
        assert!(p.x < 96.0 && LANE_TOP < p.y && p.y < 600.0, "{p:?}");
    }
    let values = rig.ctx.read_response(id().with("graph")).unwrap().rect;
    assert_eq!(
        values,
        Rect::from_min_max(pos2(98.0, LANE_TOP), pos2(800.0, 600.0))
    );
    assert_eq!(rig.view.graph.item, 0);
    rig.click(at[2]);
    assert_eq!(rig.view.graph.item, 2);
    rig.click(at[3]);
    assert_eq!(rig.view.graph.item, 3);
    // The knob on the lane's top hides it.
    rig.click(pos2(44.8, STRIP_TOP));
    assert!(!rig.view.graph.open);
    assert_eq!(rig.text_at("Velocity"), None);
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 8, step: 3 }]);
}

#[test]
fn a_touch_and_each_move_paint_the_current_rows_step_under_x() {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    // The Snare row (hits on 4 and 12): halfway up on 4, above the lane on 8 (the top), at the bottom on 12.
    rig.stroke(&[
        pos2(rig.step_x(4), lane_y(0.5)),
        pos2(rig.step_x(8), 300.0),
        pos2(rig.step_x(12), 598.0),
    ]);
    // Step 8 holds no Snare hit: nothing to send there.
    let vel = |step, value| paint(Prop::Velocity, step, value);
    assert_eq!(rig.actions, [vel(4, 127), vel(12, 1), StepAction::GraphEnd]);
    let d = (Event::DEFAULT_PAN, Event::DEFAULT_PITCH, 0);
    assert_eq!(
        rig.values(),
        [
            (0, 255, d.0, d.1, d.2),
            (2, 255, d.0, d.1, d.2),
            (1, 127, d.0, d.1, d.2),
            (1, 1, d.0, d.1, d.2),
        ]
    );
    // The bar of 127: from the bottom, 2 px into its step and 3 px short of its end (step 4 is 195..219).
    let ink = tinted(&rig, Prop::Velocity, 127, false);
    let bar = Rect::from_min_max(
        pos2(197.0, (600.0 - 296.0 * 127.0 / 255.0f32).round()),
        pos2(216.0, 600.0),
    );
    assert!(
        rig.rects(ink).contains(&bar),
        "{bar:?} in {:?}",
        rig.rects(ink)
    );
}

/// A pinch before any paint ends a touch that changed nothing: its own undo re-syncs.
#[test]
fn a_pinch_before_any_lane_paint_is_a_touch_that_changed_nothing() {
    use egui::TouchPhase::*;
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    rig.actions.clear();
    let before = rig.channel().lanes[0].clips[0].events.clone();
    // No Snare hit on step 0: the touch-down paints nothing.
    let x = rig.step_x(0);
    rig.touch(0.1, Start, pos2(x, lane_y(0.5)));
    let second = |phase| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(2),
        phase,
        pos: pos2(400.0, 100.0),
        force: None,
    };
    rig.frame(vec![second(Start)]);
    assert_eq!(rig.actions, [StepAction::GraphCancel(before.clone())]);
    rig.frame(vec![second(End)]);
    rig.touch(0.1, End, pos2(x, lane_y(0.5)));
    rig.frame(vec![]);
    assert_eq!(rig.actions, [StepAction::GraphCancel(before)]);
}

#[test]
fn a_second_finger_puts_a_lane_drag_back_and_nothing_paints_until_the_lift() {
    // The pinch's first finger on the lane must not paint on.
    use egui::TouchPhase::*;
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    rig.actions.clear();
    let before = rig.channel().lanes[0].clips[0].events.clone();
    let x = rig.step_x(4);
    rig.touch(0.1, Start, pos2(x, lane_y(0.5)));
    rig.touch(0.1, Move, pos2(x, lane_y(1.0)));
    assert_eq!(rig.values()[2].1, 255);
    // The second finger lands on the grid: the drag ends with the hit as it was.
    let second = |phase| egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(2),
        phase,
        pos: pos2(400.0, 100.0),
        force: None,
    };
    rig.frame(vec![second(Start)]);
    assert_eq!(rig.channel().lanes[0].clips[0].events, before);
    rig.touch(0.1, Move, pos2(x, lane_y(0.2)));
    rig.frame(vec![second(End)]);
    rig.touch(0.1, Move, pos2(x, lane_y(0.9)));
    rig.touch(0.1, End, pos2(x, lane_y(0.9)));
    rig.frame(vec![]);
    assert_eq!(rig.channel().lanes[0].clips[0].events, before);
    let vel = |value| paint(Prop::Velocity, 4, value);
    assert_eq!(
        rig.actions,
        [vel(127), vel(255), StepAction::GraphCancel(before)]
    );
    // The next touch paints again.
    rig.touch(0.1, Start, pos2(x, lane_y(0.5)));
    rig.touch(0.1, End, pos2(x, lane_y(0.5)));
    assert_eq!(rig.values()[2].1, 127);
}

#[test]
fn panning_and_pitch_snap_to_the_centre_in_a_1_u_zone_and_repeat_stacks_up_to_7() {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    let (four, twelve) = (rig.step_x(4), rig.step_x(12));
    for (item, top) in [("Panning", 254), ("Pitch", 65534), ("Repeat", 7)] {
        let at = rig.text_at(item).unwrap();
        rig.click(at);
        rig.actions.clear();
        // Step 4 within half a U of the centre; step 12 from inside the lane to above it.
        rig.click(pos2(four, LANE_MID + 15.0));
        rig.stroke(&[pos2(twelve, 400.0), pos2(twelve, 0.0)]);
        let prop = graph_lane::DRUM_PROPS[rig.view.graph.item];
        assert_eq!(rig.actions.last(), Some(&StepAction::GraphEnd));
        assert!(
            rig.actions.contains(&paint(prop, 12, top)),
            "{:?}",
            rig.actions
        );
    }
    // Panning and Pitch there are 127 and 0x7FFF; Repeat, which has no dead zone, round(7 · 133/296) = 3.
    assert_eq!(rig.values()[2], (1, 255, 127, 0x7FFF, 3));
    assert_eq!(rig.values()[3], (1, 255, 254, 65534, 7));
    // A centred value draws its dot on the band's centre.
    let at = rig.text_at("Panning").unwrap();
    rig.click(at);
    let ink = tinted(&rig, Prop::Pan, 127, false);
    let dot = rig.shapes.iter().any(|c| {
        matches!(&c.shape, egui::Shape::Circle(d) if d.fill == ink && d.center == pos2(206.5, LANE_MID) && d.radius == 4.0)
    });
    assert!(dot, "no dot");
}

#[test]
fn while_steps_are_selected_only_they_take_the_paint_and_a_gesture_that_changes_nothing_clears_them()
 {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    let snare = rig.cell(1, 12);
    rig.click_with(snare, PointerButton::Secondary);
    assert_eq!(rig.view.selection.len(), 1);
    rig.stroke(&[
        pos2(rig.step_x(4), lane_y(0.5)),
        pos2(rig.step_x(12), 598.0),
    ]);
    let velocity: Vec<(u8, u8)> = rig.values().iter().map(|v| (v.0, v.1)).collect();
    assert_eq!(velocity, [(0, 255), (2, 255), (1, 255), (1, 1)]);
    assert_eq!(rig.view.selection.len(), 1);
    // On step 4 again: nothing eligible changes, so the selection goes.
    rig.click(pos2(rig.step_x(4), lane_y(0.5)));
    assert!(rig.view.selection.is_empty());
}

#[test]
fn a_focus_loss_ends_a_lane_drag_and_a_stale_press_paints_nothing() {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let (a, b) = (rig.step_x(0), rig.step_x(4));
    rig.frame(vec![egui::Event::PointerMoved(pos2(a, lane_y(0.5)))]);
    rig.press(pos2(a, lane_y(0.5)), PointerButton::Primary, true);
    rig.frame(vec![egui::Event::WindowFocused(false)]);
    rig.frame(vec![
        egui::Event::WindowFocused(true),
        egui::Event::PointerMoved(pos2(b, lane_y(0.0))),
    ]);
    rig.press(pos2(b, lane_y(0.0)), PointerButton::Primary, false);
    let first = StepAction::Graph {
        pad: 0,
        prop: Prop::Velocity,
        step: 0,
        value: 127,
    };
    assert_eq!(rig.actions, [first, StepAction::GraphEnd]);
}

#[test]
fn a_lane_touch_that_changes_no_hit_or_comes_back_re_syncs_as_its_own_undo() {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    // No Snare hit on step 0, and step 4's at its own value: no edit. A touch that changes nothing undoes its own
    // snapshot, and that undo re-syncs: the hits as they were, for the host.
    let before = rig.channel().lanes[0].clips[0].events.clone();
    rig.stroke(&[
        pos2(rig.step_x(0), 0.0 + LANE_TOP + 10.0),
        pos2(rig.step_x(0), 598.0),
    ]);
    assert_eq!(rig.actions, [StepAction::GraphCancel(before.clone())]);
    let at = rig.text_at("Panning").unwrap();
    rig.click(at);
    rig.actions.clear();
    rig.click(pos2(rig.step_x(4), LANE_MID));
    assert_eq!(rig.actions, [StepAction::GraphCancel(before.clone())]);
    // Up to 254 and back to the centre: the drag ends with the hits as they were, so as a touch that changed nothing.
    rig.actions.clear();
    let x = rig.step_x(4);
    rig.stroke(&[pos2(x, LANE_MID), pos2(x, 0.0), pos2(x, LANE_MID)]);
    let pan = |value| paint(Prop::Pan, 4, value);
    let back = StepAction::GraphCancel(before);
    assert_eq!(rig.actions, [pan(254), pan(127), back]);
    assert_eq!(rig.values()[2].2, 127);
}

// The ruler and its region menu.

impl Rig {
    /// On the ruler, over the start of step `step`.
    fn ruler(&self, step: usize) -> Pos2 {
        let l = self.layout();
        pos2(l.x0 + step as f32 * l.step_w, 24.0)
    }

    /// A long press over step `from` (held 0.3 s), dragged to step `to` and released: the range.
    fn select(&mut self, from: usize, to: usize) {
        let (a, b) = (self.ruler(from), self.ruler(to));
        self.frame(vec![egui::Event::PointerMoved(a)]);
        self.press(a, PointerButton::Primary, true);
        self.frame_after(0.31, vec![]);
        self.frame(vec![egui::Event::PointerMoved(b)]);
        self.release(b);
    }

    /// Selects steps `from..to` and taps the `+` under the released edge: the region menu opens.
    fn region(&mut self, from: usize, to: usize) {
        self.select(from, to);
        let plus = self.text_at("+").expect("the + button");
        self.click(plus);
    }

    fn pick(&mut self, label: &str) {
        let at = self.text_at(label).expect(label);
        self.click(at);
    }

    /// The actions other than the ruler's play-loop updates.
    fn edits(&self) -> Vec<StepAction> {
        (self.actions.iter())
            .filter(|a| !matches!(a, StepAction::PlayLoop(_)))
            .cloned()
            .collect()
    }
}

const STEPS_4_8: crate::ui::region::TimeRange = crate::ui::region::TimeRange {
    start: 4 * STEP_TICKS,
    end: 8 * STEP_TICKS,
};

#[test]
fn the_top_strip_has_a_ruler_over_the_steps() {
    // A `background` fill, then a `menuFill` baseline t tall at bottom − 2t across the full width (t = 2 at U = 32).
    let rig = Rig::new(pattern());
    let l = rig.layout();
    let colors = theme_by_id(0).colors;
    let strip = Rect::from_min_max(pos2(l.grid_left(), 0.0), pos2(SCREEN.right(), 48.0));
    let bg = rig.rects(colors.background.to_color32());
    assert!(bg.contains(&strip), "{bg:?}");
    let base = Rect::from_min_max(pos2(l.grid_left(), 44.0), pos2(SCREEN.right(), 46.0));
    assert!(rig.rects(colors.menu_fill.to_color32()).contains(&base));
}

#[test]
fn a_ruler_range_is_the_play_loop_and_its_menu_edits_the_pattern_in_one_action() {
    let mut rig = Rig::new(pattern());
    rig.select(4, 8);
    // The clip starts at song tick 0: content ticks are song ticks.
    assert_eq!(rig.song.play_loop, Some((128, 256)));
    assert_eq!(rig.edits(), []);
    let plus = rig.text_at("+").unwrap();
    rig.click(plus);
    assert!(rig.text_at("Trim selection").is_some());
    assert_eq!(rig.text_at("Snap to Scale"), None);
    rig.pick("Delete space");
    // Snare 4 goes; Snare 12 moves to step 8.
    assert_eq!(rig.hits(), on(&[(0, 0), (2, 2), (1, 8)]));
    assert_eq!(
        rig.edits(),
        [StepAction::Region {
            op: crate::ui::region::RegionOp::DeleteSpace,
            range: STEPS_4_8,
            playhead: 0
        }]
    );
    assert_eq!(rig.song.play_loop, None);
}

#[test]
fn a_ruler_range_wholly_before_the_song_start_is_no_play_loop() {
    let mut channel = pattern();
    // Five beats into its one-bar content: content tick 0 plays at song tick −128 (Preview's span).
    channel.lanes[0].clips[0].offset_beats = 5.0;
    let mut rig = Rig::new(channel);
    // Steps 0..4 are song ticks −128..0: empty from tick 0, so no range.
    rig.select(0, 4);
    assert_eq!(rig.song.play_loop, None);
}

#[test]
fn the_drum_rulers_playhead_items_follow_the_transports_playhead_in_the_clip() {
    let mut rig = Rig::new(pattern());
    // The range is picked with the playhead knob out of the way (it takes the presses on its square).
    rig.view.playhead = 13 * u64::from(STEP_TICKS);
    rig.region(4, 8);
    // At step 5 (and a bit), inside the range: the playhead items are off.
    rig.view.playhead = 5 * u64::from(STEP_TICKS) + 9;
    rig.frame(vec![]);
    rig.pick("Copy to playhead");
    assert_eq!(rig.edits(), []);
    // At step 13: `Copy to playhead` inserts the range's hits there.
    rig.view.playhead = 13 * u64::from(STEP_TICKS);
    rig.frame(vec![]);
    rig.pick("Copy to playhead");
    assert_eq!(rig.hits(), on(&[(0, 0), (2, 2), (1, 4), (1, 12), (1, 13)]));
}

#[test]
fn a_tap_on_the_drum_ruler_asks_for_the_playhead_on_a_step() {
    let mut rig = Rig::new(pattern());
    let at = rig.ruler(6) + egui::vec2(10.0, 0.0);
    rig.click(at);
    assert_eq!(
        rig.edits(),
        [StepAction::SetPlayhead(6 * u64::from(STEP_TICKS))]
    );
}

#[test]
fn a_tap_on_an_offset_clips_drum_ruler_seeks_to_the_song_tick_it_shows() {
    // At song tick 512, two beats into its one-bar pattern: content tick 0 sits at song tick s = 256 (the ruler maps
    // song ticks), so step 4 (content tick 128) is song tick 384.
    let mut channel = pattern();
    let clip = &mut channel.lanes[0].clips[0];
    (clip.start_tick, clip.offset_beats) = (512, 2.0);
    let mut rig = Rig::new(channel);
    let at = rig.ruler(4) + egui::vec2(10.0, 0.0);
    rig.click(at);
    assert_eq!(rig.edits(), [StepAction::SetPlayhead(384)]);
}

#[test]
fn a_region_edit_drops_the_step_selection() {
    let mut rig = Rig::new(pattern());
    let at = rig.cell(1, 4);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selection(), [(1, 4)]);
    rig.region(0, 4);
    rig.pick("Insert space");
    assert_eq!(rig.hits(), on(&[(0, 4), (2, 6), (1, 8), (1, 16)]));
    assert_eq!(rig.selection(), []);
}

// Hit colour, pop-in and remembered values.

/// A Drums channel with one Snare hit on step 4 at the page laws' hand values: velocity 102 (t 0.4), pan 190
/// (t ≈ 0.496), pitch 0xBFFF (t ≈ 0.5), repeat 4 (t 0.5).
fn odd_hit() -> Channel {
    let mut channel = drums();
    let c = channel.lanes[0].add_clip(0, 4.0);
    c.events.push(Event::Drum {
        pos: 4 * STEP_TICKS,
        len_beats: 0.25,
        pad: 1,
        velocity: 102,
        pan: 190,
        value: 0xBFFF,
        repeat: 4,
    });
    channel
}

/// The odd hit's value on each page, in `DRUM_PROPS` order.
const ODD_VALUES: [u16; 4] = [102, 190, 0xBFFF, 4];

/// The hit colour on `prop`'s page for `value`: the note tint over the channel colour, or over its selected
/// (whitened) colour, the bottom stop pulled as the page says.
fn tinted(rig: &Rig, prop: Prop, value: u16, selected: bool) -> Color32 {
    let theme = theme_by_id(0);
    let mut c = channel_color(rig.channel().hue, &theme);
    if selected {
        c = crate::ui::clips::mix(c, theme.colors.text, 0.5);
    }
    let stops = graph_lane::tint(&theme.colors, c, prop.pull());
    graph_lane::tint_at(&stops, prop.tint_t(value))
}

impl Rig {
    /// The fill of the hit drawn in cell (`row`, `step`): the tall rect inside it.
    fn hit_fill(&self, row: usize, step: usize) -> Option<Color32> {
        let cell = self.layout().cell(row, step);
        self.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.rect != cell
                    && cell.contains_rect(r.rect)
                    && r.rect.height() > r.rect.width() =>
            {
                Some(r.fill)
            }
            _ => None,
        })
    }

    /// Shows graph-lane page `item` (the lane open or not) and draws a frame.
    fn page(&mut self, item: usize) {
        self.view.graph.item = item;
        self.frame(vec![]);
    }
}

#[test]
fn a_hit_takes_the_note_tint_with_t_from_the_graph_lane_page_and_a_selected_one_the_selected_tint()
{
    let mut rig = Rig::new(odd_hit());
    for (item, (prop, value)) in graph_lane::DRUM_PROPS
        .into_iter()
        .zip(ODD_VALUES)
        .enumerate()
    {
        rig.page(item);
        let want = tinted(&rig, prop, value, false);
        assert_eq!(rig.hit_fill(1, 4), Some(want), "{prop:?}");
    }
    let at = rig.cell(1, 4);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.selection(), [(1, 4)]);
    for (item, (prop, value)) in graph_lane::DRUM_PROPS
        .into_iter()
        .zip(ODD_VALUES)
        .enumerate()
    {
        rig.page(item);
        let want = tinted(&rig, prop, value, true);
        assert_eq!(rig.hit_fill(1, 4), Some(want), "selected {prop:?}");
    }
}

#[test]
fn the_graph_lanes_bars_take_the_hit_colour_too() {
    let mut rig = Rig::new(odd_hit());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    let values = rig.ctx.read_response(id().with("graph")).unwrap().rect;
    for (item, (prop, value)) in graph_lane::DRUM_PROPS
        .into_iter()
        .zip(ODD_VALUES)
        .enumerate()
    {
        rig.page(item);
        let want = tinted(&rig, prop, value, false);
        let bars = rig.rects(want);
        assert!(
            bars.iter().any(|b| values.contains_rect(*b)),
            "{prop:?}: no bar in {want:?}"
        );
    }
}

#[test]
fn the_pop_in_starts_at_1_and_falls_0_9_or_0_6_a_frame_to_0_under_0_01() {
    let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
    assert_eq!(pop_in(STEP_POP, 0.0), 1.0);
    assert!(near(pop_in(STEP_POP, 1.0 / 60.0), 0.9));
    assert!(near(pop_in(STEP_POP, 2.0 / 60.0), 0.81));
    // 0.9^43 ≈ 0.0108 still shows; 0.9^44 ≈ 0.0097 is 0.
    assert!(pop_in(STEP_POP, 43.0 / 60.0) > 0.01);
    assert_eq!(pop_in(STEP_POP, 44.0 / 60.0), 0.0);
    // The free note-grid's 0.6: 0.6^9 ≈ 0.0101, 0.6^10 ≈ 0.006.
    assert!(pop_in(0.6, 9.0 / 60.0) > 0.0);
    assert_eq!(pop_in(0.6, 10.0 / 60.0), 0.0);
}

#[test]
fn a_tapped_hit_pops_in_from_text_to_its_colour() {
    let theme = theme_by_id(0);
    let text = theme.colors.text.to_color32();
    let mut rig = one_bar();
    let at = rig.cell(2, 5);
    rig.click(at);
    // Its first frame: all `text`.
    assert_eq!(rig.hit_fill(2, 5), Some(text));
    // 0.1 s on: 0.9 per 60 Hz frame, six frames.
    rig.frame(vec![]);
    let full = tinted(&rig, Prop::Velocity, 255, false);
    let a = pop_in(STEP_POP, 0.1);
    assert!(0.5 < a && a < 0.55, "{a}");
    assert_eq!(rig.hit_fill(2, 5), Some(full.lerp_to_gamma(text, a)));
    // 0.8 s on, under 0.01: the hit colour.
    for _ in 0..7 {
        rig.frame(vec![]);
    }
    assert_eq!(rig.hit_fill(2, 5), Some(full));
    // A filled row does not pop.
    rig.menu(3, 1, 0);
    assert_eq!(rig.hit_fill(3, 0), Some(full));
}

#[test]
fn remembered_values_take_a_hits_length_at_least_the_minimum_its_velocity_and_pan() {
    let mut r = Remembered::default();
    assert_eq!(
        r,
        Remembered {
            len_beats: 0.25,
            velocity: 255,
            pan: 127,
        }
    );
    // `drum` is 1/4 beat long, velocity 100, pan 10.
    r.select(&drum(1, 0), 0.5);
    let want = Remembered {
        len_beats: 0.5,
        velocity: 100,
        pan: 10,
    };
    assert_eq!(r, want);
    r.select(&drum(1, 0), 1.0 / 128.0);
    assert_eq!(r.len_beats, 0.25);
    // A graph edit: Velocity and Panning only.
    r.graph(Prop::Velocity, 40);
    r.graph(Prop::Pan, 254);
    r.graph(Prop::Pitch, 3);
    r.graph(Prop::Repeat, 2);
    let want = Remembered {
        len_beats: 0.25,
        velocity: 40,
        pan: 254,
    };
    assert_eq!(r, want);
    // Not a drum hit: nothing.
    let point = Event::Automation {
        pos: 0,
        value: 0.5,
        control: true,
    };
    r.select(&point, 1.0);
    assert_eq!(r, want);
}

#[test]
fn a_hit_alone_in_the_selection_is_remembered_and_new_hits_take_its_velocity_and_pan() {
    let mut rig = Rig::new(odd_hit());
    rig.scroll_to(0.0);
    let default = Remembered::default();
    assert_eq!(rig.view.remembered, default);
    // A new hit: the defaults, one step long.
    let at = rig.cell(0, 0);
    rig.click(at);
    assert_eq!(rig.values()[0], (0, 255, 127, 0x7FFF, 0));
    // Selected alone (a right click), the Snare hit is remembered.
    let at = rig.cell(1, 4);
    rig.click_with(at, PointerButton::Secondary);
    let snare = Remembered {
        len_beats: 0.25,
        velocity: 102,
        pan: 190,
    };
    assert_eq!(rig.view.remembered, snare);
    // A tap on an empty step only clears the selection; the next one adds a hit with the remembered values.
    let at = rig.cell(2, 7);
    rig.click(at);
    assert_eq!(rig.selection(), []);
    rig.click(at);
    assert!(rig.values().contains(&(2, 102, 190, 0x7FFF, 0)));
    // Two hits selected remember nothing; a touch-down that makes one of them the selection remembers it.
    let at = rig.cell(0, 0);
    rig.click_with(at, PointerButton::Secondary);
    assert_eq!(rig.view.remembered.velocity, 255);
    rig.view.remembered = default;
    rig.boxed(rig.cell(0, 0), rig.cell(1, 4));
    assert_eq!(rig.selection(), [(0, 0), (1, 4)]);
    assert_eq!(rig.view.remembered, default);
    rig.click(rig.cell(2, 7));
    let at = rig.cell(1, 4);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selection(), [(1, 4)]);
    assert_eq!(rig.view.remembered, snare);
}

#[test]
fn a_lone_copy_is_remembered() {
    let mut rig = Rig::new(odd_hit());
    let at = rig.cell(1, 4);
    rig.click_with(at, PointerButton::Secondary);
    rig.view.remembered = Remembered::default();
    let copy = rig.text_at("Copy").unwrap();
    rig.click(copy);
    assert_eq!(rig.selection(), [(1, 5)]);
    assert_eq!(rig.view.remembered.velocity, 102);
}

#[test]
fn graph_lane_velocity_and_panning_edits_are_remembered_for_new_hits() {
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let header = rig.header(1);
    rig.click(header);
    let four = rig.step_x(4);
    rig.click(pos2(four, lane_y(0.5)));
    assert_eq!(rig.view.remembered.velocity, 127);
    for (item, v) in [("Panning", 1.0), ("Pitch", 0.0), ("Repeat", 1.0)] {
        let at = rig.text_at(item).unwrap();
        rig.click(at);
        rig.click(pos2(four, lane_y(v)));
    }
    assert_eq!(rig.view.remembered.pan, 254);
    assert_eq!(rig.view.remembered.velocity, 127);
    let at = rig.cell(3, 9);
    rig.click(at);
    assert!(rig.values().contains(&(3, 127, 254, 0x7FFF, 0)));
}

#[test]
fn fill_hits_are_velocity_255_centred_and_0_long() {
    let mut c = clip(4.0);
    fill(&mut c, 2, 4, SIG);
    let want: Vec<Event> = [0, 4, 8, 12]
        .map(|step| Event::Drum {
            pos: step * STEP_TICKS,
            len_beats: 0.0,
            pad: 2,
            velocity: 255,
            pan: 127,
            value: 0x7FFF,
            repeat: 0,
        })
        .to_vec();
    assert_eq!(c.events, want);
}

#[test]
fn a_tapped_hit_takes_the_lanes_swing_on_odd_steps() {
    // At 4/4 odd sixteenths move 0.125 · s beat, 8 ticks at s = 0.5; even ones stay.
    let mut channel = drums();
    channel.lanes[0].swing = 0.5;
    channel.lanes[0].add_clip(0, 4.0);
    let mut rig = Rig::new(channel);
    for step in [3, 2] {
        let at = rig.cell(1, step);
        rig.click(at);
    }
    assert_eq!(positions(&rig), [2 * STEP_TICKS, 3 * STEP_TICKS + 8]);
    // Still in its step: a tap there takes it away.
    let at = rig.cell(1, 3);
    rig.click(at);
    assert_eq!(positions(&rig), [2 * STEP_TICKS]);
}

#[test]
fn a_pop_stays_with_its_clip() {
    // Clip 0 empty, clip 1 with a Hat hit (pad 2) on step 5: the cell a tap on clip 0 pops in.
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 4.0);
    tap(channel.lanes[0].add_clip(512, 4.0), 2, 5);
    let mut rig = Rig::new(channel);
    let at = rig.cell(2, 5);
    rig.click(at);
    let text = theme_by_id(0).colors.text.to_color32();
    assert_eq!(rig.hit_fill(2, 5), Some(text));
    // Another clip: its hit in that cell shows its own colour, with no pop.
    rig.view.clip = Some(2);
    rig.frame(vec![]);
    assert_eq!(
        rig.hit_fill(2, 5),
        Some(tinted(&rig, Prop::Velocity, 255, false))
    );
    // Back on clip 0 the pop is gone too.
    rig.view.clip = Some(1);
    rig.frame(vec![]);
    assert_eq!(
        rig.hit_fill(2, 5),
        Some(tinted(&rig, Prop::Velocity, 255, false))
    );
}

#[test]
fn the_graph_knobs_box_wins_over_a_cut_off_header_and_the_first_selector_row() {
    // As in the piano roll, and in both drum modes: where the knob's 2×2 U touch box overlaps the graph list's first
    // item or a row header cut off at the rows' bottom, however thin the header's visible part, the knob takes the tap.
    let mut rig = Rig::new(pattern());
    rig.kit = twelve();
    // Row 9 shows only its top 8 points, 592..600, inside the knob's box (x 12.8..76.8, y 568..600).
    rig.set_row_view(RowView {
        pitch: 2.0,
        scroll: 1.0,
    });
    rig.click(pos2(44.8, 596.0));
    assert!(rig.view.graph.open);
    assert_eq!((rig.view.selected, rig.view.fold.open), (0, false));
    // The lane's top is at 300: the knob's box (y 268..332) covers row 4's cut-off header above it (272..300) ...
    rig.click(pos2(44.8, 280.0));
    assert!(!rig.view.graph.open);
    assert_eq!(rig.view.selected, 0);
    // ... and the list's first item (`Velocity`, from 304) below it.
    rig.view.graph.item = 2;
    rig.click(pos2(44.8, 596.0));
    rig.click(pos2(44.8, 316.0));
    assert_eq!((rig.view.graph.open, rig.view.graph.item), (false, 2));
    // Outside the box both take their taps.
    rig.click(pos2(44.8, 596.0));
    rig.click(pos2(90.0, 316.0));
    assert_eq!((rig.view.graph.open, rig.view.graph.item), (true, 0));
    rig.click(pos2(44.8, 316.0));
    rig.click(pos2(90.0, 596.0));
    assert_eq!((rig.view.graph.open, rig.view.selected), (false, 9));
}

#[test]
fn the_step_grid_has_tdiv_byte_0_steps_a_beat_and_a_new_pattern_is_a_byte_1_bar() {
    // d = TDIV byte 0 steps per beat; a step starts at round(i·128/d).
    for (d, b, want) in [(3u8, 3u8, [43, 85, 128]), (6, 2, [21, 43, 64])] {
        let mut rig = Rig::new(drums());
        rig.song.time_sig = TimeSig {
            subdivision: d,
            beats_per_bar: b,
        };
        // `+Clip`'s new pattern: one bar of b beats (`ui::clips::add_clip`).
        let bar = rig.song.time_sig.bar_beats();
        crate::ui::clips::add_clip(&mut rig.song.channels[CH].lanes[0], 0.0, 1.0, bar);
        rig.view.clip = Some(1);
        rig.frame(vec![]);
        for step in 1..=3 {
            let at = rig.cell(0, step);
            rig.click(at);
        }
        let clip = &rig.channel().lanes[0].clips[0];
        assert_eq!(clip.loop_len_beats, f64::from(b), "d {d}");
        let pos: Vec<u32> = clip.events.iter().map(Event::pos).collect();
        assert_eq!(pos, want, "d {d}");
        // A hit is one step long.
        assert!(
            clip.events
                .iter()
                .all(|e| matches!(*e, Event::Drum { len_beats, .. }
            if len_beats == 1.0 / f64::from(d)))
        );
    }
}

#[test]
fn a_subdivision_of_0_swings_as_1() {
    // `TDIV` byte 0 = 0 counts as d = 1: odd beats move by 0.125·s beat.
    let sig = TimeSig {
        subdivision: 0,
        beats_per_bar: 4,
    };
    let mut c = clip(4.0);
    toggle(&mut c, 0, 1, 255, 127, (sig, 1.0));
    assert_eq!(c.events[0].pos(), 128 + 16);
    let mut c = clip(4.0);
    c.events = vec![drum(0, 0), drum(0, 128)];
    swing(&mut c, &[0, 128], sig, 0.5);
    assert_eq!(
        c.events.iter().map(Event::pos).collect::<Vec<_>>(),
        [0, 136]
    );
}

#[test]
fn copied_off_grid_hits_stay_in_their_step_and_rotation_goes_by_the_nearest_step() {
    // At d = 3 a step is 42.67 ticks: tick 42 is nearest step 1 (llround(start·d)), one tick before its start (43).
    // Rotate moves it to step 2's start (85); a copy keeps its offset, one tick before step 2 (84), so it stays on its
    // step.
    let sig = TimeSig {
        subdivision: 3,
        beats_per_bar: 3,
    };
    let mut c = clip(3.0);
    c.events = vec![drum(0, 42)];
    rotate(&mut c, 0, true, (sig, 0.0));
    assert_eq!(c.events[0].pos(), 85);
    assert_eq!(cell_of(&c.events[0], sig), Some((0, 2)));
    let mut c = clip(3.0);
    c.events = vec![drum(0, 42)];
    assert_eq!(copy_steps(&mut c, &[(0, 1)], sig), [(0, 2)]);
    assert_eq!(
        c.events.iter().map(Event::pos).collect::<Vec<_>>(),
        [42, 84]
    );
}

#[test]
fn fills_rotations_and_copies_follow_the_subdivision() {
    for (d, starts) in [
        (3u8, [0, 43, 85, 128, 171, 213]),
        (6, [0, 21, 43, 64, 85, 107]),
    ] {
        let sig = TimeSig {
            subdivision: d,
            beats_per_bar: 2,
        };
        let pos = |c: &Clip| c.events.iter().map(Event::pos).collect::<Vec<_>>();
        // A two-beat loop holds 2d steps; each hit lies in its own step.
        let mut c = clip(2.0);
        fill(&mut c, 0, 1, sig);
        assert_eq!(pos(&c).len(), 2 * usize::from(d), "d {d}");
        assert_eq!(pos(&c)[..6], starts, "d {d}");
        let cells: Vec<_> = c.events.iter().filter_map(|e| cell_of(e, sig)).collect();
        assert_eq!(
            cells,
            (0..2 * usize::from(d)).map(|s| (0, s)).collect::<Vec<_>>()
        );
        // Rotating right moves each hit a step on; the last wraps to step 0.
        let mut c = clip(2.0);
        toggle(&mut c, 0, 1, 255, 127, (sig, 0.0));
        toggle(&mut c, 0, 2 * usize::from(d) - 1, 255, 127, (sig, 0.0));
        rotate(&mut c, 0, true, (sig, 0.0));
        assert_eq!(pos(&c), [0, starts[2]], "d {d}");
        // A copy lands on the next step's start.
        let mut c = clip(2.0);
        toggle(&mut c, 0, 1, 255, 127, (sig, 0.0));
        assert_eq!(copy_steps(&mut c, &[(0, 1)], sig), [(0, 2)]);
        assert_eq!(pos(&c), [starts[1], starts[2]], "d {d}");
    }
}

#[test]
fn a_mouse_held_still_300_ms_on_a_row_header_opens_its_menu_but_released_at_250_ms_is_a_tap() {
    let mut rig = Rig::new(pattern());
    let at = rig.header(3);
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    rig.frame_after(0.1, vec![]);
    rig.frame_after(
        0.15,
        vec![egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    rig.frame(vec![]);
    assert_eq!(rig.view.selected, 3, "a tap makes the row current");
    assert!(rig.area("row menu").is_none());
    let at = rig.header(1);
    rig.frame(vec![egui::Event::PointerMoved(at)]);
    rig.press(at, PointerButton::Primary, true);
    rig.frame_after(0.2, vec![]);
    assert!(rig.area("row menu").is_none(), "not before 300 ms");
    rig.frame_after(0.1, vec![]);
    rig.release(at);
    assert_eq!(rig.view.selected, 1);
    assert!(rig.area("row menu").is_some());
    assert!(matches!(
        rig.view.overlay,
        Some(Overlay::Menu { row: 1, .. })
    ));
}

// The step grid's drawing, its time axis, the loop's growth and the ruler.

impl Rig {
    /// A wheel turn by `delta` points with the pointer at `at`.
    fn wheel(&mut self, at: Pos2, delta: egui::Vec2) {
        self.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers: Modifiers::NONE,
            },
        ]);
    }

    /// The fill of the rect painted exactly at `rect` last, if any.
    fn fill_at(&self, rect: Rect) -> Option<Color32> {
        self.shapes.iter().rev().find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
            _ => None,
        })
    }

    fn loop_beats(&self) -> f64 {
        self.channel().lanes[0].clips[0].loop_len_beats
    }
}

/// A one-bar clip at 4/4, from a fresh rig.
fn one_bar() -> Rig {
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 4.0);
    Rig::new(channel)
}

#[test]
fn cells_run_from_content_beat_0_with_1_px_gaps_at_ppp_1_and_2() {
    for ppp in [1.0, 2.0] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
        let px = 1.0 / ppp;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 448.0));
        // A step 24.3 points wide, content beat 0 a third of a beat left of the grid (x 96).
        let l = Layout::new(rect, 4, 1, &m, 0.0, RowView::default()).with_time(63.6, 24.3);
        // Content beat 0 is off the grid's left: its first visible step is the one under x 96.
        assert_eq!(l.visible_steps(), 1..31, "ppp {ppp}");
        for step in 1..29 {
            let (a, b) = (l.cell(2, step), l.cell(2, step + 1));
            assert_eq!(
                b.left() - a.right(),
                px,
                "a 1 px gap after step {step}, ppp {ppp}"
            );
            assert_eq!(
                a.left(),
                (((63.6 + 24.3 * step as f32) * ppp).round()) / ppp
            );
            assert_eq!(
                l.cell(3, step).top() - a.bottom(),
                px,
                "a 1 px gap under the row"
            );
            for v in [a.left(), a.right(), a.top(), a.bottom()] {
                assert_eq!((v * ppp).fract(), 0.0, "{a:?} on device pixels");
            }
        }
        // The grid's touch area is every visible step, past the loop too, from content beat 0.
        assert_eq!(l.hit(pos2(700.0, 120.0)), Some((1, 26)));
        assert_eq!(l.hit(pos2(90.0, 120.0)), None, "the header column");
        let l = l.with_time(200.0, 24.3);
        assert_eq!(l.visible_steps(), 0..25);
        assert_eq!(l.hit(pos2(150.0, 120.0)), None, "left of content beat 0");
    }
}

#[test]
fn an_active_step_is_inset_2_px_and_trunc_r_4_tall_with_its_dashes_at_ppp_1_and_2() {
    for ppp in [1.0, 2.0] {
        let px = 1.0 / ppp;
        // A cell 24 points wide (P = 25 points with its gap), in a 64.3-point row.
        let cell = Rect::from_min_max(pos2(120.0, 112.0), pos2(144.0 - px, 176.0));
        let hit = active_rect(cell, 64.3, ppp);
        assert_eq!(hit.left(), 120.0 + 2.0 * px);
        assert_eq!(hit.right(), 144.0 - 3.0 * px, "P − 5 px wide");
        assert_eq!(hit.top(), 112.0 + 2.0 * px);
        assert_eq!(
            hit.height(),
            (64.3 * ppp - 4.0).trunc() * px,
            "trunc(R − 4) px"
        );
        // t = ceil(U/16) px: 2 points at ppp 1, 4 px = 2 points at ppp 2. Repeat 2: three dashes 3t apart.
        let t = 2.0;
        let d: Vec<Rect> = dashes(hit, 2, t).collect();
        assert_eq!(d.len(), 3);
        for (i, dash) in d.iter().enumerate() {
            assert_eq!(
                dash.x_range(),
                Rangef::new(hit.left() + 2.0 * t, hit.right() - 2.0 * t)
            );
            assert_eq!(dash.top(), hit.top() + 2.0 * t + 3.0 * t * i as f32);
            assert_eq!(dash.height(), t);
        }
        assert_eq!(dashes(hit, 0, t).count(), 1);
    }
}

#[test]
fn cells_alternate_per_beat_in_the_loop_and_are_all_control_fill_past_it() {
    let rig = Rig::new(pattern());
    let colors = theme_by_id(0).colors;
    let (dark, light) = (
        colors.control_fill.to_color32(),
        colors.module_fill.to_color32(),
    );
    let l = rig.layout();
    // 3.03 U a beat: the 702-point grid shows 7.24 beats, 28 whole steps and a part.
    assert_eq!(l.visible_steps(), 0..29);
    for step in l.visible_steps() {
        let want = if step < 16 && (step / 4) % 2 == 1 {
            light
        } else {
            dark
        };
        assert_eq!(rig.fill_at(l.cell(3, step)), Some(want), "step {step}");
    }
    // The hits: on their cells, inset; the Kick on step 0 in the hit colour with a `background` dash.
    let hit = active_rect(l.cell(0, 0), l.pitch, 1.0);
    assert_eq!(
        rig.fill_at(hit),
        Some(tinted(&rig, Prop::Velocity, 255, false))
    );
    let dash = dashes(hit, 0, 2.0).next().unwrap();
    assert_eq!(rig.fill_at(dash), Some(colors.background.to_color32()));
    // Nothing is drawn as a line: every rect in the grid's rows is a cell, a hit or a dash.
    assert!(
        rig.fill_at(Rect::from_x_y_ranges(
            l.cell(0, 3).right()..=l.cell(0, 4).left(),
            l.cell(0, 0).y_range()
        ))
        .is_none()
    );
}

#[test]
fn a_swung_hit_is_drawn_on_its_own_step() {
    let mut channel = drums();
    channel.lanes[0].swing = 0.5;
    channel.lanes[0].add_clip(0, 4.0);
    let mut rig = Rig::new(channel);
    let at = rig.cell(0, 5);
    rig.click(at);
    assert_eq!(
        rig.hits(),
        on(&[(0, 5)])
            .into_iter()
            .map(|(p, s, _)| (p, s, 8))
            .collect::<Vec<_>>()
    );
    rig.frame_after(2.0, vec![]);
    let l = rig.layout();
    let hit = active_rect(l.cell(0, 5), l.pitch, 1.0);
    assert_eq!(
        rig.fill_at(hit),
        Some(tinted(&rig, Prop::Velocity, 255, false))
    );
}

#[test]
fn the_graph_lanes_columns_take_the_loops_shading_with_start_at_most_l() {
    // b = 3, a one-bar loop: L = 3 beats, 12 steps. The column starting exactly at L (step 12, beat 3, odd) is in range
    // there (moduleFill), where the grid's cell is past it (controlFill).
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 3.0);
    let mut rig = Rig::new(channel);
    rig.song.time_sig = TimeSig {
        subdivision: 4,
        beats_per_bar: 3,
    };
    rig.open_lane();
    let colors = theme_by_id(0).colors;
    let (dark, light) = (
        colors.control_fill.to_color32(),
        colors.module_fill.to_color32(),
    );
    let l = rig.layout();
    let column = |step: usize| {
        (rig.shapes.iter()).find_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.rect.x_range() == l.span(l.x0, l.step_w, step) && r.rect.top() == LANE_TOP =>
            {
                Some(r.fill)
            }
            _ => None,
        })
    };
    assert_eq!(column(11), Some(dark), "beat 2");
    assert_eq!(column(12), Some(light), "start = L is in range on the lane");
    assert_eq!(column(13), Some(dark));
    assert_eq!(
        rig.fill_at(l.cell(0, 12)),
        Some(dark),
        "start = L is past the loop on the grid"
    );
}

#[test]
fn a_tap_past_the_loop_grows_it_to_b_2_k_in_one_action_and_deleting_shrinks_it() {
    let mut rig = one_bar();
    // A tap at beat 5 (end 5.25): one action, and L = 8.
    let at = rig.cell(0, 20);
    rig.click(at);
    assert_eq!(rig.actions, [StepAction::Toggle { pad: 0, step: 20 }]);
    assert_eq!(rig.loop_beats(), 8.0);
    // The next frame shades it in range (beat 5, odd: light).
    let light = theme_by_id(0).colors.module_fill.to_color32();
    rig.frame(vec![]);
    let l = rig.layout();
    assert_eq!(rig.fill_at(l.cell(3, 20)), Some(light));
    // A hit at beat 8 makes L = 16 (scrolled to see it).
    rig.scroll_to(4.0);
    let at = rig.cell(1, 32);
    rig.click(at);
    assert_eq!(rig.loop_beats(), 16.0);
    // A tap on a hit deletes it: L drops to the smallest b·2^k that still holds the rest, at least b, with no
    // hysteresis.
    let at = rig.cell(1, 32);
    rig.click(at);
    assert_eq!(rig.loop_beats(), 8.0);
    rig.scroll_to(0.0);
    let at = rig.cell(0, 20);
    rig.click(at);
    assert_eq!(rig.loop_beats(), 4.0);
    assert_eq!(rig.hits(), []);
    assert_eq!(rig.actions.len(), 4, "one action, one undo step, per tap");
}

#[test]
fn a_swung_tap_on_the_last_step_doubles_the_loop() {
    // Swing 50 %: step 15 starts 8 ticks late and stays a step long, so it ends past the bar.
    let mut channel = drums();
    channel.lanes[0].swing = 0.5;
    channel.lanes[0].add_clip(0, 4.0);
    let mut rig = Rig::new(channel);
    let at = rig.cell(0, 15);
    rig.click(at);
    assert_eq!(rig.loop_beats(), 8.0);
    // Without Swing it stays a bar.
    let mut rig = one_bar();
    let at = rig.cell(0, 15);
    rig.click(at);
    assert_eq!(rig.loop_beats(), 4.0);
}

#[test]
fn the_playlist_length_follows_l_until_the_clip_is_resized() {
    // The visible length if > 0, else L.
    let mut c = clip(4.0);
    assert_eq!(c.visible_len_beats, 0.0, "a new clip");
    tap(&mut c, 0, 20);
    c.fit_loop(SIG.bar_beats());
    assert_eq!(c.visible_ticks(), 8.0 * 128.0);
    let mut c = clip(4.0);
    c.visible_len_beats = 6.0;
    tap(&mut c, 0, 20);
    c.fit_loop(SIG.bar_beats());
    assert_eq!((c.loop_len_beats, c.visible_ticks()), (8.0, 6.0 * 128.0));
}

#[test]
fn the_time_zoom_keeps_a_beat_between_3_03_u_and_10_u_at_d_4() {
    // [(0.1/U)·4/d, (0.33/U)·4/d] beats per px.
    let u = 32.0;
    let w = |k| drum_grid::edit::beat_w(u, 4, step_k(k));
    assert_eq!(w(0.01), 10.0 * u);
    assert!((w(1.0) - u / 0.33).abs() < 1e-4);
    assert_eq!(
        drum_grid::edit::beat_w(u, 8, step_k(0.01)),
        20.0 * u,
        "d = 8: steps keep their width"
    );
    // The wheel over the ruler zooms the time within them, the beat under the pointer kept.
    let mut rig = one_bar();
    let ruler = pos2(300.0, 24.0);
    rig.wheel(ruler, vec2(0.0, 300.0));
    // zoom ← zoom·(1 − 0.001·300): k × 0.7.
    let k = rig.clip_view().k;
    assert!((k - 0.33 * 0.7).abs() < 1e-5, "{k}");
    for _ in 0..10 {
        rig.wheel(ruler, vec2(0.0, 600.0));
    }
    assert_eq!(rig.clip_view().k, 0.1);
    assert!((rig.layout().step_w - 10.0 * u / 4.0).abs() < 1e-4);
    for _ in 0..10 {
        rig.wheel(ruler, vec2(0.0, -600.0));
    }
    assert_eq!(rig.clip_view().k, 0.33);
}

#[test]
fn a_new_clip_opens_at_3_03_u_a_beat_from_its_earliest_hit_and_lowest_row() {
    // A four-bar clip (room to scroll: 2L = 32 beats) whose earliest hit is pad 3 at beat 5.
    let mut channel = drums();
    let c = channel.lanes[0].add_clip(0, 16.0);
    // Pad 11 makes 12 rows, with room to scroll.
    c.events = vec![drum(3, 5 * 128), drum(11, 9 * 128)];
    let rig = Rig::new(channel);
    let view = rig.clip_view();
    assert_eq!(view.k, 0.33);
    assert!(
        (rig.layout().step_w * 4.0 - 32.0 / 0.33).abs() < 1e-3,
        "3.03 U a beat"
    );
    assert_eq!(view.x, 5.0);
    assert_eq!(view.rows.scroll, 3.0 * 2.0, "row 3 on top");
    // A new empty clip's first tap: the view stays where it was.
    let mut rig = one_bar();
    let at = rig.cell(1, 6);
    rig.click(at);
    assert_eq!(rig.hits(), on(&[(1, 6)]));
    assert_eq!((rig.clip_view().x, rig.clip_view().rows.scroll), (0.0, 0.0));
}

#[test]
fn the_time_scrolls_over_0_to_2l_less_the_view() {
    // One bar of 4/4: 8 beats of content, 7.24 in view: 0.76 to scroll.
    let mut rig = one_bar();
    let grid = rig.cell(1, 6);
    rig.wheel(grid, vec2(-2000.0, 0.0));
    let max = 8.0 - 702.0 / f64::from(32.0f32 / 0.33);
    assert!(
        (rig.clip_view().x - max).abs() < 1e-4,
        "{:?}",
        rig.clip_view()
    );
    rig.wheel(grid, vec2(2000.0, 0.0));
    assert_eq!(rig.clip_view().x, 0.0);
    // A drag on the grid scrolls the time too (sideways past the calm zone).
    let from = rig.cell(1, 9);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    for i in 1..=4 {
        rig.frame(vec![egui::Event::PointerMoved(
            from - vec2(15.0 * i as f32, 0.0),
        )]);
    }
    rig.press(from - vec2(60.0, 0.0), PointerButton::Primary, false);
    // The scroller: x stays still for the first move (15 points, inside its 0.5 U calm zone), then follows 1:1: 45
    // points.
    let x = 45.0 / f64::from(32.0f32 / 0.33);
    assert!(
        (rig.clip_view().x - x).abs() < 1e-4,
        "{:?}",
        rig.clip_view()
    );
    assert!(rig.actions.is_empty(), "a drag adds no hit");
    // A bar of 2/4 is 4 beats of content: the view spans it, nothing scrolls.
    let mut rig = one_bar();
    rig.song.time_sig.beats_per_bar = 2;
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 2.0;
    rig.frame(vec![]);
    rig.wheel(rig.cell(1, 6), vec2(-2000.0, 0.0));
    assert_eq!(rig.clip_view().x, 0.0);
}

#[test]
fn follow_puts_the_folded_playhead_at_a_quarter_of_the_view_while_playing_a_longer_loop() {
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 16.0).events = vec![drum(0, 0)];
    let mut rig = Rig::new(channel);
    let visible = 702.0 / f64::from(32.0f32 / 0.33);
    rig.view.follow = true;
    // Stopped: no follow.
    rig.view.playhead = 10 * 128;
    rig.frame(vec![]);
    assert_eq!(rig.clip_view().x, 0.0);
    // Playing beat 26 of the song: folded into the 16-beat loop, beat 10.
    rig.view.playing = true;
    rig.view.playhead = 26 * 128;
    rig.frame(vec![]);
    assert!(
        (rig.clip_view().x - (10.0 - 0.25 * visible)).abs() < 1e-4,
        "{:?}",
        rig.clip_view()
    );
    // A drag sideways does not move it meanwhile.
    let from = rig.cell(1, 50);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    for i in 1..=4 {
        rig.frame(vec![egui::Event::PointerMoved(
            from + vec2(30.0 * i as f32, 0.0),
        )]);
    }
    rig.press(from + vec2(120.0, 0.0), PointerButton::Primary, false);
    assert!((rig.clip_view().x - (10.0 - 0.25 * visible)).abs() < 1e-4);
    // Off with the setting, or for a loop that fits the view.
    rig.view.follow = false;
    rig.view.playhead = 3 * 128;
    rig.frame(vec![]);
    assert!((rig.clip_view().x - (10.0 - 0.25 * visible)).abs() < 1e-4);
    let mut rig = one_bar();
    (rig.view.follow, rig.view.playing, rig.view.playhead) = (true, true, 3 * 128);
    rig.frame(vec![]);
    assert_eq!(rig.clip_view().x, 0.0);
}

#[test]
fn the_rulers_bar_numbers_are_song_bars_and_its_ticks_sit_over_the_baseline() {
    // A one-bar clip at song bar 5: its content beat 4 is song bar 6's line.
    let mut channel = drums();
    channel.lanes[0].add_clip(4 * 512, 4.0);
    let rig = Rig::new(channel);
    let l = rig.layout();
    let colors = theme_by_id(0).colors;
    let badge = |n: &str| {
        (rig.shapes.iter()).find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text() == n
                    && t.galley.job.sections[0].format.color == colors.menu_fill.to_color32() =>
            {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
    };
    let beat = l.step_w * 4.0;
    let six = badge("6").expect("bar 6");
    assert!((six.x - (l.x0 + 4.0 * beat)).abs() < 1.0, "{six:?}");
    // "5" sits on X, within 6t: skipped. No content bar numbers.
    assert_eq!(badge("5"), None);
    assert_eq!(badge("2"), None);
    // Ticks: a beat-level one 2t wide and 0.3h tall, bottom at bottom − 3t (48 − 6); a step-level one t wide, 0.2h.
    let ink = rig.rects(colors.menu_fill.to_color32());
    let near = |want: Rect| {
        ink.iter()
            .any(|r| (r.min - want.min).length() < 1e-3 && (r.max - want.max).length() < 1e-3)
    };
    let beat1 = l.x0 + beat;
    assert!(near(Rect::from_x_y_ranges(
        beat1 - 2.0..=beat1 + 2.0,
        42.0 - 14.4..=42.0
    )));
    let step1 = l.x0 + l.step_w;
    assert!(near(Rect::from_x_y_ranges(
        step1 - 1.0..=step1 + 1.0,
        42.0 - 9.6..=42.0
    )));
}

#[test]
fn the_scroll_indicator_shows_the_view_over_the_loop() {
    // A four-bar clip, scrolled 2 beats in: 7.24 of 16 beats in view.
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 16.0);
    let mut rig = Rig::new(channel);
    rig.scroll_to(2.0);
    let ink = channel_color(rig.channel().hue, &theme_by_id(0)).to_color32();
    let w = 702.0;
    let visible = w / (32.0 / 0.33);
    let want = Rect::from_min_size(
        pos2(98.0 + 2.0 * w / 16.0, 0.0),
        vec2(w * visible / 16.0, 2.0),
    );
    let bars = rig.rects(ink);
    assert!(bars.iter().any(|r| (r.min - want.min).length() < 1e-3 && (r.size() - want.size()).length() < 1e-3), "{bars:?}");
    // A loop the view spans has none.
    let rig = one_bar();
    assert!(
        rig.rects(ink)
            .iter()
            .all(|r| r.height() != 2.0 || r.top() != 0.0)
    );
}

#[test]
fn a_ruler_tap_past_the_loop_end_does_nothing() {
    let mut rig = one_bar();
    let at = rig.ruler(20) + vec2(10.0, 0.0);
    rig.click(at);
    assert_eq!(rig.edits(), []);
    let at = rig.ruler(15) + vec2(10.0, 0.0);
    rig.click(at);
    assert_eq!(
        rig.edits(),
        [StepAction::SetPlayhead(15 * u64::from(STEP_TICKS))]
    );
}

#[test]
fn the_playhead_line_sits_at_the_folded_playhead_stopped_or_playing_down_to_the_last_row() {
    // The pattern's 4 rows of 64 from y 48: the line ends at 304. t = 2: the line 4 wide, its shadow 6.
    let mut rig = Rig::new(pattern());
    let colors = theme_by_id(0).colors;
    // Stopped at song tick 512 + 6 steps: folded into the one-bar loop, step 6.
    rig.view.playhead = 512 + 6 * u64::from(STEP_TICKS);
    rig.frame(vec![]);
    let l = rig.layout();
    // On the device pixel nearest the playhead's x (ppp 1).
    let x = (l.x0 + 6.0 * l.step_w).round();
    let line = Rect::from_x_y_ranges(x - 2.0..=x + 2.0, 48.0..=304.0);
    assert!(
        rig.rects(colors.text.to_color32()).contains(&line),
        "{:?}",
        rig.rects(colors.text.to_color32())
    );
    let shadow = Rect::from_x_y_ranges(x - 3.0..=x + 3.0, 48.0..=304.0);
    assert_eq!(
        rig.fill_at(shadow),
        Some(colors.background.to_color32().gamma_multiply(0.3))
    );
    // Playing, the same.
    rig.view.playing = true;
    rig.frame(vec![]);
    assert!(rig.rects(colors.text.to_color32()).contains(&line));
}

#[test]
fn the_knob_shows_stopped_plays_on_a_tap_and_a_drop_seeks_within_the_loop() {
    let mut rig = one_bar();
    let text = theme_by_id(0).colors.text.to_color32();
    let disc = |rig: &Rig| {
        (rig.shapes.iter()).find_map(|s| match &s.shape {
            // Not a bar badge (radius 6t = 12).
            egui::Shape::Circle(c) if c.fill == text && c.radius > 20.0 => {
                Some((c.center, c.radius))
            }
            _ => None,
        })
    };
    // Stopped at 0: a disc of radius 1.35·48/2 − 3 on the ruler's middle at content beat 0, past the t gap.
    let (center, r) = disc(&rig).expect("the knob");
    assert_eq!(center, pos2(98.0, 24.0));
    assert!((r - (1.35 * 48.0 / 2.0 - 3.0)).abs() < 1e-4);
    // A tap plays.
    rig.click(center);
    assert!(rig.view.play);
    // A drag past the loop end drops on it: s + L = 512.
    let to = rig.ruler(24);
    rig.frame(vec![egui::Event::PointerMoved(center)]);
    rig.press(center, PointerButton::Primary, true);
    for i in 1..=4 {
        rig.frame(vec![egui::Event::PointerMoved(
            center + (to - center) * i as f32 / 4.0,
        )]);
    }
    rig.press(to, PointerButton::Primary, false);
    assert_eq!(rig.edits().last(), Some(&StepAction::Seek(512)));
    // Playing, it fades out: only the line shows.
    rig.view.playing = true;
    for _ in 0..60 {
        rig.frame(vec![]);
    }
    assert_eq!(disc(&rig), None);
}

#[test]
fn while_the_current_rows_header_is_held_its_row_is_drawn_last_and_grown() {
    let mut rig = Rig::new(pattern());
    let bg = theme_by_id(0).colors.background.to_color32();
    let l = rig.layout();
    let band = l.cell(0, 0).y_range();
    let header = rig.header(0);
    rig.frame(vec![egui::Event::PointerMoved(header)]);
    rig.press(header, PointerButton::Primary, true);
    rig.frame_after(0.2, vec![]);
    // The row's band in `background` across the grid, taller than the row, then its cells over it.
    let grown = (rig.shapes.iter()).position(|s| {
        matches!(&s.shape, egui::Shape::Rect(r) if r.fill == bg && r.rect.left() == l.grid_left() && r.rect.height() > band.span() + 2.0)
    });
    let grown = grown.expect("the grown band");
    let cell = (rig.shapes.iter()).rposition(|s| {
        matches!(&s.shape, egui::Shape::Rect(r) if r.rect.x_range() == l.cell(0, 3).x_range() && r.rect.height() > band.span() + 2.0)
    });
    assert!(
        cell.is_some_and(|c| c > grown),
        "the grown cells over the band"
    );
    // Released, it snaps back.
    rig.press(header, PointerButton::Primary, false);
    rig.frame(vec![]);
    let l = rig.layout();
    assert!(rig.fill_at(l.cell(0, 3)).is_some());
}

#[test]
fn another_clip_takes_no_fling_from_the_last_one() {
    // A fling of one clip's rows does not move the next clip's.
    let mut channel = pattern();
    channel.lanes[0].add_clip(512, 4.0);
    let mut rig = Rig::new(channel);
    rig.kit = twelve();
    rig.frame(vec![]);
    let from = pos2(400.0, 500.0);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    for k in 1..=3 {
        let to = from - vec2(0.0, 40.0 * k as f32);
        rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(to)]);
    }
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: from - vec2(0.0, 120.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    let flying = rig.row_view().scroll;
    rig.frame_after(1.0 / 60.0, vec![]);
    assert_ne!(rig.row_view().scroll, flying, "the first clip flings");
    rig.view.clip = Some(2);
    for _ in 0..10 {
        rig.frame_after(1.0 / 60.0, vec![]);
    }
    assert_eq!(rig.row_view().scroll, 0.0);
}

// The keymap's editing and zoom actions (`ui/edit_keys.rs`).

#[test]
fn the_editing_keys_copy_cut_delete_and_paste_steps_and_enter_adds_one_at_the_touched_step() {
    let cmd = Modifiers::COMMAND;
    let key =
        |rig: &mut Rig, key, mods| rig.frame(crate::ui::edit_keys::tests::tap(key, mods).into());
    let long = |rig: &mut Rig, row, step| {
        let at = rig.cell(row, step);
        rig.hold(at);
        rig.release(at);
    };
    let mut rig = Rig::new(pattern());
    long(&mut rig, 2, 2);
    key(&mut rig, egui::Key::C, cmd);
    assert_eq!(hit_clipboard(&rig.ctx).len(), 1);
    key(&mut rig, egui::Key::Escape, Modifiers::NONE);
    assert_eq!(rig.selection(), []);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // The long press on (1, 12) puts the cursor there: the copied step lands on step 12, on its own pad.
    long(&mut rig, 1, 12);
    key(&mut rig, egui::Key::V, cmd);
    assert!(matches!(
        rig.actions[..],
        [StepAction::PasteSteps { step: 12, .. }]
    ));
    assert_eq!(rig.selection(), [(2, 12)]);
    key(&mut rig, egui::Key::Delete, Modifiers::NONE);
    assert_eq!(
        rig.actions[1],
        StepAction::DeleteSteps {
            cells: vec![(2, 12)]
        }
    );
    // Enter on a step that holds a hit adds nothing.
    key(&mut rig, egui::Key::Enter, Modifiers::NONE);
    assert_eq!(rig.actions.len(), 2);
    // Cut, then Enter on the emptied step: the hit is back.
    long(&mut rig, 1, 4);
    key(&mut rig, egui::Key::X, cmd);
    assert_eq!(
        rig.actions[2],
        StepAction::DeleteSteps {
            cells: vec![(1, 4)]
        }
    );
    assert_eq!(rig.hits(), on(&[(0, 0), (2, 2), (1, 12)]));
    key(&mut rig, egui::Key::Enter, Modifiers::NONE);
    assert_eq!(rig.actions[3], StepAction::Toggle { pad: 1, step: 4 });
    assert_eq!(rig.hits(), on(&[(0, 0), (2, 2), (1, 4), (1, 12)]));
}

#[test]
fn scrolled_off_a_step_boundary_nothing_paints_into_the_header_column() {
    // One bar scrolled to beat 0.1: content beat 0 sits 9.7 points left of the grid. The cells and the Kick on step 0
    // are cut at the grid's left; the playhead line at the header column's right edge, a t gap before it (its left half
    // shows there at beat 0).
    let mut rig = Rig::new(pattern());
    rig.scroll_to(0.1);
    let l = rig.layout();
    assert!(l.x0 < l.grid_left() - 9.0, "{l:?}");
    // The playhead 13 ticks in, just inside the grid: its line's left half in the gap, its shadow reaching over the
    // column's edge. (In the gap itself it is off the grid, so not drawn.)
    rig.view.playhead = 13;
    rig.frame(vec![]);
    let l = rig.layout();
    let colors = theme_by_id(0).colors;
    let cells = [
        colors.control_fill.to_color32(),
        colors.module_fill.to_color32(),
        tinted(&rig, Prop::Velocity, 255, false),
    ];
    let line = [
        colors.text.to_color32(),
        colors.background.to_color32().gamma_multiply(0.3),
    ];
    let rows_bottom = l.row_top() + 4.0 * l.pitch;
    let spill = |inks: &[Color32], edge: f32| -> Vec<Rect> {
        (rig.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if inks.contains(&r.fill)
                        && r.rect.left() > 0.0
                        && r.rect.left() < edge
                        && r.rect.top() < rows_bottom
                        && s.clip_rect.left() < edge =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        spill(&[&cells[..], &line[..]].concat(), l.column_right()),
        [],
        "painted into the header column"
    );
    assert_eq!(spill(&cells, l.grid_left()), [], "painted into the t gap");
    // They are drawn, cut: step 0's cell at the grid, the line at the column.
    let cut = |want: Rect, edge: f32| {
        (rig.shapes.iter()).any(|s| {
            matches!(&s.shape, egui::Shape::Rect(r) if r.rect == want) && s.clip_rect.left() == edge
        })
    };
    assert!(cut(l.cell(3, 0), l.grid_left()));
    let x = (l.x0 + 13.0 / 128.0 * 4.0 * l.step_w).round();
    assert!(
        x - 2.0 >= l.column_right() && x - 3.0 < l.column_right(),
        "{x}"
    );
    let line = Rect::from_x_y_ranges(x - 2.0..=x + 2.0, l.top..=rows_bottom);
    assert!(cut(line, l.column_right()));
}

#[test]
fn the_grid_starts_a_t_gap_right_of_the_header_column_where_its_zones_start() {
    // GL = x + HW + ceil(U/16) px. The layout left the gap out while its zones had it, so the drawing and the touches
    // were t apart.
    for ppp in [1.0, 1.5, 2.0] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 448.0));
        let l = Layout::new(rect, 4, 16, &m, 192.0, RowView::default());
        let gl = m.u(HEADER_W) + 192.0 + m.t();
        assert_eq!(
            (l.grid_left(), l.x0, l.grid().left()),
            (gl, gl, gl),
            "{ppp}"
        );
        assert_eq!(zones(&l).touch[0].left(), gl);
        assert!((l.x0 + 16.0 * l.step_w - 800.0).abs() < 1e-3, "{l:?}");
    }
}

/// The playhead line's rect (2t = 4 points of `text`, from the grid top) and its clip, if the last frame drew it.
fn playhead_line(shapes: &[egui::epaint::ClippedShape], top: f32) -> Option<(Rect, Rect)> {
    let text = theme_by_id(0).colors.text.to_color32();
    shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Rect(r) if r.fill == text && r.rect.width() == 4.0 && r.rect.top() == top => {
            Some((r.rect, s.clip_rect))
        }
        _ => None,
    })
}

#[test]
fn at_beat_0_the_playhead_lines_left_half_shows_in_the_t_gap() {
    // The playhead line's z-order: the line is cut at the header column's right edge, so at beat 0 its left half falls
    // in the t gap and shows (the piano roll's rule).
    let mut rig = Rig::new(pattern());
    rig.view.playhead = 0;
    rig.frame(vec![]);
    let column = 3.0 * 32.0;
    let (line, clip) = playhead_line(&rig.shapes, 48.0).expect("the line");
    assert_eq!(line.x_range(), Rangef::new(column, column + 4.0));
    assert_eq!(clip.left(), column);
}

#[test]
fn the_lane_is_f_of_the_editor_below_its_top_margin_and_its_knob_drags_f_on_that_height() {
    // The piano roll's law here too: height max(f·H, 2 U), H = the editor below its top window margin (0 for a bottom
    // editor: the SWING strip counts); a knob drag sets f = 1 − (y − top)/H.
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let mut lane = GraphLane::default();
    lane.open = true;
    let (above, strip) = lane_split(rect, &metrics(), &lane);
    assert_eq!(strip, Some(rect.with_min_y(300.0)));
    assert_eq!(above, rect.with_max_y(300.0));
    let mut rig = Rig::new(pattern());
    rig.open_lane();
    let knob = pos2(KNOB.x, 300.0);
    rig.frame(vec![egui::Event::PointerMoved(knob)]);
    rig.press(knob, PointerButton::Primary, true);
    for y in [280.0, 260.0, 240.0] {
        rig.frame(vec![egui::Event::PointerMoved(pos2(knob.x, y))]);
    }
    rig.press(pos2(knob.x, 240.0), PointerButton::Primary, false);
    assert!(
        (rig.view.graph.f - 0.6).abs() < 1e-6,
        "{}",
        rig.view.graph.f
    );
}

/// The `menuFill` disc of the record icon (icon 5) near `center`, if the last frame painted one.
fn record_icon(rig: &Rig, center: Pos2) -> bool {
    let menu = theme_by_id(0).colors.menu_fill.to_color32();
    (rig.shapes.iter()).any(|s| {
        matches!(&s.shape, egui::Shape::Circle(c) if c.fill == menu && c.center.distance(center) < 1.5)
    })
}

#[test]
fn the_knob_shows_the_record_icon_while_rec_is_armed() {
    // Icon 5 (REC) while record is armed, else 3 (play).
    let mut rig = one_bar();
    let knob = pos2(98.0, 24.0);
    assert!(!record_icon(&rig, knob), "the play icon");
    rig.view.armed = true;
    rig.frame(vec![]);
    assert!(record_icon(&rig, knob));
}

#[test]
fn a_ruler_region_on_a_clip_at_bar_2_edits_the_content_ticks_it_shows() {
    // The clip at song bar 2 (s = 512). Steps 4–8 on the ruler are song ticks 640–768, the play loop; the region item
    // edits content ticks 128–256.
    let mut channel = pattern();
    channel.lanes[0].clips[0].start_tick = 512;
    let mut rig = Rig::new(channel);
    rig.select(4, 8);
    assert_eq!(rig.song.play_loop, Some((640, 768)));
    let plus = rig.text_at("+").expect("the + button");
    rig.click(plus);
    rig.pick("Delete space");
    assert_eq!(
        rig.edits(),
        [StepAction::Region {
            op: crate::ui::region::RegionOp::DeleteSpace,
            range: STEPS_4_8,
            playhead: 0
        }]
    );
    // Snare 4 goes; Snare 12 moves to step 8.
    assert_eq!(rig.hits(), on(&[(0, 0), (2, 2), (1, 8)]));
}

#[test]
fn a_repeat_2_hit_paints_three_dashes() {
    // Repeat r paints r + 1 `background` dashes, each 3t below the last.
    let mut channel = drums();
    let mut hit = drum(1, 4 * STEP_TICKS);
    Prop::Repeat.set(&mut hit, 2);
    channel.lanes[0].add_clip(0, 4.0).events = vec![hit];
    let rig = Rig::new(channel);
    let l = rig.layout();
    let bg = theme_by_id(0).colors.background.to_color32();
    let hit = active_rect(l.cell(1, 4), l.pitch, 1.0);
    let painted = |r: Rect| rig.fill_at(r) == Some(bg);
    let d: Vec<Rect> = dashes(hit, 2, 2.0).collect();
    assert_eq!(d.len(), 3);
    assert!(d.iter().all(|&r| painted(r)), "{d:?}");
    assert!(!painted(dashes(hit, 3, 2.0).nth(3).unwrap()), "no fourth");
}

impl Rig {
    /// Drags the playhead knob at `from` to `to` in four moves and lets go.
    fn drag_knob(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.press(from, PointerButton::Primary, true);
        for i in 1..=4 {
            let at = from + (to - from) * i as f32 / 4.0;
            self.frame(vec![egui::Event::PointerMoved(at)]);
        }
        self.press(to, PointerButton::Primary, false);
    }
}

#[test]
fn a_knob_drop_clamps_to_the_views_left_edge_and_snaps_to_the_step() {
    // A four-bar clip scrolled to beat 2, stopped at beat 3: dropped over the header column, the knob seeks to the
    // view's left edge, beat 2.
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 16.0);
    let mut rig = Rig::new(channel);
    rig.scroll_to(2.0);
    rig.view.playhead = 3 * 128;
    rig.frame(vec![]);
    let l = rig.layout();
    let knob = pos2(l.x0 + 12.0 * l.step_w, 24.0);
    rig.drag_knob(knob, pos2(40.0, 24.0));
    assert_eq!(rig.edits().last(), Some(&StepAction::Seek(256)));
    // Dropped 5.6 steps in, it snaps to step 6.
    let mut rig = one_bar();
    let l = rig.layout();
    rig.drag_knob(pos2(l.x0, 24.0), pos2(l.x0 + 5.6 * l.step_w, 24.0));
    assert_eq!(rig.edits().last(), Some(&StepAction::Seek(6 * 32)));
}

#[test]
fn the_playhead_folds_into_the_loop_from_before_the_clip_too() {
    // s = 2048, L = 512: song tick 100 is (100 − 2048) mod 512 = 100.
    assert_eq!(ruler::folded(100, 2048, 512.0), 100.0);
    assert_eq!(ruler::folded(2048 + 600, 2048, 512.0), 88.0);
    assert_eq!(ruler::folded(5, 0, 0.0), 0.0);
}

#[test]
fn a_hits_step_is_llround_start_d_so_a_late_swung_hit_shows_on_the_next_step() {
    // A hit's step is llround(start·d). At d = 8 Swing 0.6 starts an odd step 0.075 beat late (10 ticks of its 16,
    // rounded to 1/128 beat): nearest the next step.
    let sig = TimeSig {
        subdivision: 8,
        beats_per_bar: 4,
    };
    let mut c = clip(4.0);
    // The new hit is on cell 4.
    assert_eq!(toggle(&mut c, 0, 3, 255, 127, (sig, 0.6)), Some((0, 4)));
    assert_eq!(c.events[0].pos(), 3 * 16 + 10);
    assert_eq!(cell_of(&c.events[0], sig), Some((0, 4)));
    assert_eq!(cells_at(&c, sig), [(0, 4)]);
    // A copy keeps its offset from its step (6 ticks before step 5's start), and so its step.
    assert_eq!(copy_steps(&mut c, &[(0, 4)], sig), [(0, 5)]);
    assert_eq!(c.events[1].pos(), 5 * 16 - 6);
    // A tap on its cell takes it away.
    assert_eq!(toggle(&mut c, 0, 4, 255, 127, (sig, 0.6)), None);
    assert_eq!(cells_at(&c, sig), [(0, 5)]);
    // Swing 0.2 (3 ticks late) keeps a hit on its own step.
    let mut c = clip(4.0);
    toggle(&mut c, 0, 3, 255, 127, (sig, 0.2));
    assert_eq!(cell_of(&c.events[0], sig), Some((0, 3)));
}

/// The cells `edit::cells` marks at `sig`, sorted.
fn cells_at(c: &Clip, sig: TimeSig) -> Vec<(u8, usize)> {
    let mut marks: Vec<_> = edit::cells(c, sig).into_keys().collect();
    marks.sort_unstable();
    marks
}

/// A four-bar clip with the Kick on beat 0 and the Snare on beat 12.
fn four_bars() -> Channel {
    let mut channel = drums();
    channel.lanes[0].add_clip(0, 16.0).events = vec![drum(0, 0), drum(1, 48 * STEP_TICKS)];
    channel
}

#[test]
fn a_quick_drag_on_the_ruler_scrolls_the_time_and_seeks_nothing() {
    // A ruler press is the scroller's until its long press (`claim`): moved sideways before 0.3 s it scrolls, and the
    // view's move cancels the press, so its release seeks nothing.
    let mut rig = Rig::new(four_bars());
    let from = pos2(500.0, 24.0);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    for i in 1..=4 {
        let at = from - vec2(30.0 * i as f32, 0.0);
        rig.frame_after(0.02, vec![egui::Event::PointerMoved(at)]);
    }
    rig.frame_after(
        0.02,
        vec![egui::Event::PointerButton {
            pos: from - vec2(120.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    rig.frame(vec![]);
    assert!(rig.clip_view().x > 0.5, "{:?}", rig.clip_view());
    assert_eq!(rig.edits(), []);
    assert_eq!(rig.song.play_loop, None);
}

#[test]
fn zooming_the_time_past_its_limit_keeps_the_beat_under_the_pointer() {
    // The scroller zooms within k's range (0.1 … 0.33) about the pointer; past the limit the anchor holds.
    let mut rig = Rig::new(four_bars());
    rig.scroll_to(2.0);
    let at = pos2(500.0, 24.0);
    let beat = |rig: &Rig| {
        let l = rig.layout();
        f64::from((at.x - l.x0) / (4.0 * l.step_w))
    };
    let before = beat(&rig);
    for _ in 0..10 {
        rig.wheel(at, vec2(0.0, 600.0));
    }
    assert_eq!(rig.clip_view().k, 0.1);
    assert!(
        (beat(&rig) - before).abs() < 1e-3,
        "{} {before}",
        beat(&rig)
    );
    for _ in 0..10 {
        rig.wheel(at, vec2(0.0, -600.0));
    }
    assert_eq!(rig.clip_view().k, 0.33);
    assert!(
        (beat(&rig) - before).abs() < 1e-3,
        "{} {before}",
        beat(&rig)
    );
}

#[test]
fn a_fling_while_following_leaves_nothing_once_playback_stops() {
    // `Options.follow`: while following, one finger does not scroll the time and no fling is left.
    let mut rig = Rig::new(four_bars());
    (rig.view.follow, rig.view.playing, rig.view.playhead) = (true, true, 10 * 128);
    rig.frame(vec![]);
    let followed = rig.clip_view().x;
    let from = rig.cell(2, 50);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.press(from, PointerButton::Primary, true);
    for i in 1..=4 {
        let at = from - vec2(40.0 * i as f32, 0.0);
        rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(at)]);
    }
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: from - vec2(160.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    rig.view.playing = false;
    for _ in 0..5 {
        rig.frame_after(1.0 / 60.0, vec![]);
    }
    assert_eq!(rig.clip_view().x, followed);
}

#[test]
fn a_shrunk_loop_lets_posx_spring_back_over_frames() {
    // In step mode, when L shrinks, posX is not clamped; the overscroll spring pulls it back, a tenth of the overshoot
    // a frame.
    let mut rig = Rig::new(four_bars());
    rig.scroll_to(10.0);
    let snare = rig.cell(1, 48);
    rig.click(snare);
    assert_eq!(rig.loop_beats(), 4.0);
    let max = 8.0 - 702.0 / f64::from(32.0f32 / 0.33);
    let x = rig.clip_view().x;
    assert!(x > 5.0 && x < 10.0, "springing: {x}");
    for _ in 0..120 {
        rig.frame(vec![]);
    }
    assert!(
        (rig.clip_view().x - max).abs() < 1e-3,
        "{:?}",
        rig.clip_view()
    );
}

/// Channel 1's one-bar lane at d = 8 with Swing 0.6, and a tap on step 3: its hit starts 10 ticks late, on step 4
/// (llround).
fn swung_at_8() -> Rig {
    let mut channel = drums();
    channel.lanes[0].swing = 0.6;
    channel.lanes[0].add_clip(0, 4.0);
    let mut rig = Rig::new(channel);
    rig.song.time_sig = TimeSig {
        subdivision: 8,
        beats_per_bar: 4,
    };
    rig.frame(vec![]);
    let at = rig.cell(0, 3);
    rig.click(at);
    assert_eq!(rig.channel().lanes[0].clips[0].events[0].pos(), 58);
    rig
}

#[test]
fn the_graph_lane_shows_and_paints_a_swung_hit_on_its_llround_column() {
    let mut rig = swung_at_8();
    rig.open_lane();
    let l = rig.layout();
    // The bar on column 4: 2 px into it, 3 px short of its end, down to the lane's bottom.
    let (start, end) = (l.snap(l.x0 + 4.0 * l.step_w), l.snap(l.x0 + 5.0 * l.step_w));
    let ink = tinted(&rig, Prop::Velocity, 255, false);
    let bar = (rig.rects(ink).into_iter()).find(|r| r.bottom() == 600.0);
    let bar = bar.expect("the hit's bar");
    assert_eq!((bar.left(), bar.right()), (start + 2.0, end - 3.0));
    // Painting column 4 changes it: an edit, closed at the release.
    rig.actions.clear();
    let x = (start + end) / 2.0;
    rig.stroke(&[pos2(x, lane_y(0.5)), pos2(x, lane_y(0.25))]);
    assert!(
        matches!(
            rig.actions[0],
            StepAction::Graph {
                pad: 0,
                prop: Prop::Velocity,
                step: 4,
                ..
            }
        ),
        "{:?}",
        rig.actions
    );
    assert_eq!(rig.actions.last(), Some(&StepAction::GraphEnd));
    assert_ne!(rig.values()[0].1, 255);
}

#[test]
fn a_swung_tap_pops_in_on_the_cell_its_hit_shows_on() {
    let rig = swung_at_8();
    assert!(
        rig.view.pops.iter().any(|p| p.0 == (0, 4)),
        "{:?}",
        rig.view.pops
    );
    // Still popping: the hit on cell 4 is blended toward `text`.
    let l = rig.layout();
    let fill = rig.fill_at(active_rect(l.cell(0, 4), l.pitch, 1.0));
    assert!(
        fill.is_some_and(|c| c != tinted(&rig, Prop::Velocity, 255, false)),
        "{fill:?}"
    );
}

#[test]
fn a_copy_stays_on_its_step_at_fractional_step_widths() {
    // d = 3 (42.67 ticks a step): tick 21 is step 0 (llround 0.49); one step on it would be 64, step 2 by llround (1.5),
    // so it lands on step 1's last tick, 63.
    let sig = TimeSig {
        subdivision: 3,
        beats_per_bar: 3,
    };
    let mut c = clip(3.0);
    c.events = vec![drum(0, 21)];
    assert_eq!(copy_steps(&mut c, &[(0, 0)], sig), [(0, 1)]);
    assert_eq!(c.events[1].pos(), 63);
    // d = 5 (25.6): tick 13 is step 1 (llround 0.51), 13 before its start (26); one step on it would be 38, step 1 by
    // llround (1.48), so it lands on step 2's first tick, 39.
    let sig = TimeSig {
        subdivision: 5,
        beats_per_bar: 4,
    };
    let mut c = clip(4.0);
    c.events = vec![drum(0, 13)];
    assert_eq!(copy_steps(&mut c, &[(0, 1)], sig), [(0, 2)]);
    assert_eq!(c.events[1].pos(), 39);
}
