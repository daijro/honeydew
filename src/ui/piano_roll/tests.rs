use eframe::egui::{Pos2, Rect, pos2};

use super::notes::*;
use crate::song::{ChannelKind, Event, Lane, LaneKind, PPQ, Song, TimeSig};

/// A note at `pos` ticks, `len` ticks long.
fn n(pos: u32, len: f64, key: u8) -> Event {
    Event::Note {
        pos,
        len_beats: len / f64::from(PPQ),
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

/// (pos, len in ticks, key) of every note.
fn spans(events: &[Event]) -> Vec<(u32, f64, u8)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::Note {
                pos,
                len_beats,
                key,
                ..
            } => Some((pos, len_beats * f64::from(PPQ), key)),
            _ => None,
        })
        .collect()
}

// ---- snap ----

#[test]
fn snap_menu_lists_the_twelve_modes_with_their_ids() {
    let labels: Vec<&str> = SnapMode::ALL.iter().map(|m| m.label()).collect();
    assert_eq!(
        labels,
        [
            "Auto", "1/6 Step", "1/4 Step", "1/3 Step", "1/2 Step", "Step", "1/6 Beat", "1/4 Beat",
            "1/3 Beat", "1/2 Beat", "Beat", "Bar"
        ]
    );
    for (id, mode) in SnapMode::ALL.iter().enumerate() {
        assert_eq!(*mode as usize, id);
    }
}

/// `TDIV` with subdivision `d` (byte 0) and `b` beats a bar (byte 1).
fn sig(d: u8, b: u8) -> TimeSig {
    TimeSig {
        subdivision: d,
        beats_per_bar: b,
    }
}

#[test]
fn snap_sizes_in_ticks() {
    let t = |m: SnapMode| m.ticks(sig(4, 4)).unwrap();
    assert_eq!(SnapMode::Auto.ticks(sig(4, 4)), None);
    assert_eq!(t(SnapMode::Step), 32.0);
    assert_eq!(t(SnapMode::HalfStep), 16.0);
    assert_eq!(t(SnapMode::QuarterStep), 8.0);
    assert!((t(SnapMode::SixthStep) - 32.0 / 6.0).abs() < 1e-9);
    assert!((t(SnapMode::ThirdStep) - 32.0 / 3.0).abs() < 1e-9);
    assert_eq!(t(SnapMode::QuarterBeat), 32.0);
    assert_eq!(t(SnapMode::HalfBeat), 64.0);
    assert!((t(SnapMode::ThirdBeat) - 128.0 / 3.0).abs() < 1e-9);
    assert!((t(SnapMode::SixthBeat) - 128.0 / 6.0).abs() < 1e-9);
    assert_eq!(t(SnapMode::Beat), 128.0);
    assert_eq!(t(SnapMode::Bar), 512.0);
    assert_eq!(SnapMode::Bar.ticks(sig(4, 3)), Some(384.0));
    // A step is 1/d beat (`TDIV` byte 0); beats and bars do not depend on it.
    for d in [3u8, 6] {
        let t = |m: SnapMode| m.ticks(sig(d, 7)).unwrap();
        let step = 128.0 / f64::from(d);
        assert!((t(SnapMode::Step) - step).abs() < 1e-9, "d {d}");
        assert!((t(SnapMode::HalfStep) - step / 2.0).abs() < 1e-9, "d {d}");
        assert!((t(SnapMode::SixthStep) - step / 6.0).abs() < 1e-9, "d {d}");
        assert_eq!(t(SnapMode::QuarterBeat), 32.0);
        assert_eq!(t(SnapMode::Beat), 128.0);
        assert_eq!(t(SnapMode::Bar), 7.0 * 128.0);
    }
}

#[test]
fn auto_snap_follows_the_zoom() {
    // U = 32: a step must be at least 0.4 U = 12.8 points wide, else a beat, else a bar.
    assert_eq!(auto_grid(64.0, 32.0, sig(4, 4)), 32.0);
    assert_eq!(auto_grid(52.0, 32.0, sig(4, 4)), 32.0);
    assert_eq!(auto_grid(51.0, 32.0, sig(4, 4)), 128.0);
    assert_eq!(auto_grid(13.0, 32.0, sig(4, 4)), 128.0);
    assert_eq!(auto_grid(12.0, 32.0, sig(4, 4)), 512.0);
    assert_eq!(auto_grid(12.0, 32.0, sig(4, 3)), 384.0);
    // A step is 1/d beat: at d = 3 it shows once a beat is 3 × 12.8 points, at d = 6 once it is 6 × 12.8.
    assert_eq!(auto_grid(39.0, 32.0, sig(3, 4)), 128.0 / 3.0);
    assert_eq!(auto_grid(38.0, 32.0, sig(3, 4)), 128.0);
    assert_eq!(auto_grid(77.0, 32.0, sig(6, 4)), 128.0 / 6.0);
    assert_eq!(auto_grid(76.0, 32.0, sig(6, 4)), 128.0);
}

#[test]
fn snap_on_uses_the_mode_or_auto_and_off_snaps_nothing() {
    let snap = Snap::default();
    assert!(snap.on);
    assert_eq!(snap.mode, SnapMode::Auto);
    assert_eq!(snap.active(32.0, sig(4, 4)), Some(32.0));
    let beat = Snap {
        on: true,
        mode: SnapMode::Beat,
    };
    assert_eq!(beat.active(32.0, sig(4, 4)), Some(128.0));
    let off = Snap { on: false, ..beat };
    assert_eq!(off.active(32.0, sig(4, 4)), None);
    // The grid itself does not depend on `on` (the floating `Snap` button quantizes to it).
    assert_eq!(off.grid(32.0, sig(4, 4)), 128.0);
}

#[test]
fn snap_round_to_the_grid() {
    let step = Some(32.0);
    assert_eq!(snap_round(70.0, step), 64.0);
    assert_eq!(snap_round(81.0, step), 96.0);
    // Off: whole ticks only.
    assert_eq!(snap_round(70.6, None), 71.0);
    // Triplet grids land on whole ticks.
    assert_eq!(snap_round(40.0, Some(128.0 / 6.0)), 43.0);
}

// ---- geometry and hit testing ----

/// Tick 0 at x = 100, key 127's row top at y = 50; 64 points a beat, 16 a row.
const G: Geom = Geom {
    origin: Pos2 { x: 100.0, y: 50.0 },
    beat_w: 64.0,
    row_h: 16.0,
};
const U: f32 = 32.0;

#[test]
fn geometry_maps_ticks_and_keys() {
    assert_eq!(G.x(0.0), 100.0);
    assert_eq!(G.x(128.0), 164.0);
    assert_eq!(G.tick(164.0), 128.0);
    assert_eq!(G.y(127), 50.0);
    assert_eq!(G.y(126), 66.0);
    assert_eq!(G.y(0), 50.0 + 127.0 * 16.0);
    assert_eq!(G.key(50.0), Some(127));
    assert_eq!(G.key(65.9), Some(127));
    assert_eq!(G.key(66.0), Some(126));
    assert_eq!(G.key(49.9), None);
    assert_eq!(G.key(50.0 + 128.0 * 16.0), None);
    assert_eq!(
        G.note_rect(&n(128, 64.0, 126)),
        Some(Rect::from_min_max(pos2(164.0, 66.0), pos2(196.0, 82.0)))
    );
}

#[test]
fn hits_tell_note_and_empty_apart() {
    // A beat-long note (64 points) on key 126, from x 164 to 228, y 66 to 82.
    let events = [n(128, 128.0, 126)];
    let hit = |x: f32, y: f32| hit_test(&G, &events, pos2(x, y));
    assert_eq!(hit(170.0, 70.0), Some(Hit::Body(0)));
    // No resize zone at the right end (only the handle resizes); the end itself is outside.
    assert_eq!(hit(227.0, 70.0), Some(Hit::Body(0)));
    assert!(matches!(hit(228.0, 70.0), Some(Hit::Empty { .. })));
    // Empty cells report the tick and key under the pointer.
    assert_eq!(
        hit(132.0, 90.0),
        Some(Hit::Empty {
            tick: 64.0,
            key: 125
        })
    );
    // Left of tick 0 or outside the keys: nothing.
    assert_eq!(hit(99.0, 70.0), None);
    assert_eq!(hit(120.0, 40.0), None);
}

#[test]
fn a_touch_binds_a_note_a_fifth_of_a_row_off_its_row_and_a_tap_rechecks_the_row() {
    // Row band [127 − key − 0.2, 128 − key + 0.2), beat in [start, end).
    // Key 126's row is y 66..82 (16 points a row: 3.2 points of band each way).
    let events = [n(128, 128.0, 126)];
    let at = |y: f32| note_at(&G, &events, pos2(170.0, y));
    assert_eq!(at(62.9), Some(0));
    assert_eq!(at(62.7), None);
    assert_eq!(at(85.1), Some(0));
    assert_eq!(at(85.3), None);
    // The tap's re-check is the exact row.
    assert!(on_note(&G, &events[0], pos2(170.0, 66.0)));
    assert!(!on_note(&G, &events[0], pos2(170.0, 65.0)));
    assert!(!on_note(&G, &events[0], pos2(170.0, 82.0)));
    // Two notes a row apart: in the margin between, the one whose row is nearer wins.
    let events = [n(128, 128.0, 126), n(128, 128.0, 125)];
    assert_eq!(note_at(&G, &events, pos2(170.0, 81.0)), Some(0));
    assert_eq!(note_at(&G, &events, pos2(170.0, 83.0)), Some(1));
}

#[test]
fn the_topmost_note_wins() {
    let events = [n(0, 256.0, 126), n(64, 64.0, 126)];
    assert_eq!(hit_test(&G, &events, pos2(150.0, 70.0)), Some(Hit::Body(1)));
    assert_eq!(hit_test(&G, &events, pos2(110.0, 70.0)), Some(Hit::Body(0)));
}

#[test]
fn a_box_selects_the_notes_it_touches() {
    let events = [n(0, 64.0, 127), n(128, 64.0, 126), n(512, 64.0, 120)];
    let rect = Rect::from_min_max(pos2(110.0, 55.0), pos2(170.0, 70.0));
    assert_eq!(in_box(&G, &events, rect), [true, true, false]);
}

// ---- edits ----

#[test]
fn added_notes_keep_the_events_sorted_and_the_mask_in_step() {
    let mut events = vec![n(0, 32.0, 60), n(128, 32.0, 62)];
    let mut sel = vec![true, false];
    let i = add_note(&mut events, &mut sel, 64.0, 32.0, 64);
    assert_eq!(i, 1);
    assert_eq!(
        spans(&events),
        [(0, 32.0, 60), (64, 32.0, 64), (128, 32.0, 62)]
    );
    assert_eq!(sel, [true, false, false]);
    // Equal positions keep insertion order: the new note goes after.
    let i = add_note(&mut events, &mut sel, 64.0, 32.0, 50);
    assert_eq!(i, 2);
    match events[2] {
        Event::Note {
            velocity, pan, key, ..
        } => assert_eq!(
            (velocity, pan, key),
            (Event::DEFAULT_VELOCITY, Event::DEFAULT_PAN, 50)
        ),
        _ => panic!("not a note"),
    }
}

#[test]
fn delete_the_selection() {
    let mut events = vec![n(64, 32.0, 62), n(128, 32.0, 64)];
    let mut sel = vec![true, false];
    assert!(remove_selected(&mut events, &mut sel));
    assert_eq!(spans(&events), [(128, 32.0, 64)]);
    assert_eq!(sel, [false]);
    assert!(!remove_selected(&mut events, &mut sel));
}

#[test]
fn moves_shift_the_chosen_notes_and_stay_in_range() {
    let orig = [n(32, 32.0, 60), n(64, 32.0, 126), n(96, 32.0, 10)];
    let which = [true, true, false];
    assert_eq!(
        spans(&shift_notes(&orig, &which, 64.0, -2)),
        [(96, 32.0, 58), (128, 32.0, 124), (96, 32.0, 10)]
    );
    // Clamped as a group: the earliest moved note stops at tick 0, the highest at key 127.
    assert_eq!(
        spans(&shift_notes(&orig, &which, -100.0, 5)),
        [(0, 32.0, 61), (32, 32.0, 127), (96, 32.0, 10)]
    );
}

#[test]
fn sorting_after_a_move_carries_the_selection_along() {
    let orig = [n(0, 32.0, 60), n(64, 32.0, 62)];
    let mut events = shift_notes(&orig, &[true, false], 128.0, 0);
    let mut sel = vec![true, false];
    sort(&mut events, &mut sel);
    assert_eq!(spans(&events), [(64, 32.0, 62), (128, 32.0, 60)]);
    assert_eq!(sel, [false, true]);
}

#[test]
fn resizes_keep_a_minimum_length() {
    let orig = [n(0, 64.0, 60), n(64, 32.0, 62)];
    assert_eq!(
        spans(&stretch_notes(&orig, &[true, true], 32.0, 32.0)),
        [(0, 96.0, 60), (64, 64.0, 62)]
    );
    assert_eq!(
        spans(&stretch_notes(&orig, &[true, false], -100.0, 32.0)),
        [(0, 32.0, 60), (64, 32.0, 62)]
    );
}

#[test]
fn copy_places_the_selection_after_itself_and_selects_the_copies() {
    let mut events = vec![n(0, 64.0, 60), n(64, 32.0, 62), n(512, 32.0, 70)];
    let mut sel = vec![true, true, false];
    // Span 0..96, rounded up to the 32-tick grid = 96.
    duplicate_selected(&mut events, &mut sel, 32.0);
    assert_eq!(
        spans(&events),
        [
            (0, 64.0, 60),
            (64, 32.0, 62),
            (96, 64.0, 60),
            (160, 32.0, 62),
            (512, 32.0, 70)
        ]
    );
    assert_eq!(sel, [false, false, true, true, false]);
}

#[test]
fn quantize_moves_starts_to_the_nearest_grid_line() {
    let mut events = vec![n(10, 32.0, 60), n(50, 32.0, 62), n(70, 32.0, 64)];
    let mut sel = vec![true, true, false];
    assert!(quantize_selected(&mut events, &mut sel, 32.0));
    assert_eq!(
        spans(&events),
        [(0, 32.0, 60), (64, 32.0, 62), (70, 32.0, 64)]
    );
    assert_eq!(sel, [true, true, false]);
    assert!(!quantize_selected(&mut events, &mut sel, 32.0));
}

#[test]
fn legato_extends_each_selected_note_to_the_next_selected_start() {
    let mut events = vec![
        n(0, 16.0, 60),
        n(0, 8.0, 64),
        n(64, 16.0, 62),
        n(200, 16.0, 70),
    ];
    let sel = vec![true, true, true, false];
    legato_selected(&mut events, &sel);
    assert_eq!(
        spans(&events),
        [
            (0, 64.0, 60),
            (0, 64.0, 64),
            (64, 16.0, 62),
            (200, 16.0, 70)
        ]
    );
}

#[test]
fn double_copies_an_octave_away_inside_the_key_range() {
    let mut events = vec![n(0, 32.0, 60), n(32, 32.0, 120)];
    let mut sel = vec![true, true];
    double_selected(&mut events, &mut sel, 12);
    assert_eq!(
        spans(&events),
        [(0, 32.0, 60), (0, 32.0, 72), (32, 32.0, 120)]
    );
    assert_eq!(sel, [true, false, true]);
}

#[test]
fn set_length_applies_to_the_selection() {
    let mut events = vec![n(0, 32.0, 60), n(32, 32.0, 62)];
    let sel = vec![false, true];
    set_length_selected(&mut events, &sel, 512.0);
    assert_eq!(spans(&events), [(0, 32.0, 60), (32, 512.0, 62)]);
}

#[test]
fn the_loop_grows_to_whole_bars_around_the_notes() {
    assert_eq!(needed_loop_beats(&[], 4), 0.0);
    assert_eq!(needed_loop_beats(&[n(0, 128.0, 60)], 4), 4.0);
    assert_eq!(needed_loop_beats(&[n(500, 32.0, 60)], 4), 8.0);
    assert_eq!(needed_loop_beats(&[n(500, 12.0, 60)], 4), 4.0);
    assert_eq!(needed_loop_beats(&[n(500, 32.0, 60)], 3), 6.0);
}

// ---- song mapping: playhead and ghost notes ----

/// Master, then two instrument channels and a drum channel. Channel 1 has a 4-beat clip at song tick 1024.
fn ghost_song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Instrument);
    song.add_channel(ChannelKind::Instrument);
    song.add_channel(ChannelKind::Drums);
    let edited = song.channels[1].lanes[0].add_clip(1024, 4.0);
    edited.events = vec![n(0, 32.0, 60)];
    // The same channel's other clip: never a ghost.
    let own = song.channels[1].lanes[0].add_clip(1100, 4.0);
    own.events = vec![n(0, 32.0, 61)];
    // Channel 2: one note inside the window, one before and one after it.
    let other = song.channels[2].lanes[0].add_clip(1000, 8.0);
    other.events = vec![n(0, 16.0, 40), n(124, 16.0, 41), n(600, 16.0, 42)];
    // A drum lane plays inside the window, but drum hits are not notes.
    let drums = song.channels[3].lanes[0].add_clip(1024, 4.0);
    drums.events = vec![Event::Drum {
        pos: 0,
        len_beats: 0.25,
        pad: 0,
        velocity: 200,
        pan: 127,
        value: 0x7FFF,
        repeat: 0,
    }];
    // An automation lane on channel 2 inside the window.
    let mut lane = Lane::new(LaneKind::Automation);
    lane.add_clip(1024, 4.0).events = vec![Event::Automation {
        pos: 0,
        value: 0.5,
        control: true,
    }];
    song.channels[2].lanes.push(lane);
    song
}

/// Every content tick, for [`ghost_notes`].
const ALL: std::ops::Range<f64> = f64::NEG_INFINITY..f64::INFINITY;

#[test]
fn ghost_notes_walk_only_the_view_whatever_the_loop_lengths() {
    // An edited clip a million beats long, and another channel's one-tick loop over a million beats: 1.28e8 ghosts a
    // frame, until memory ran out. Walking only the view, and no loop pass narrower than a pixel, bounds them.
    let mut song = Song::default();
    for _ in 0..3 {
        song.add_channel(ChannelKind::Instrument);
    }
    let edited = song.channels[1].lanes[0].add_clip(0, 1e6);
    edited.visible_len_beats = 1e6;
    let ticks = song.channels[2].lanes[0].add_clip(0, 1.0 / f64::from(PPQ));
    ticks.visible_len_beats = 1e6;
    ticks.events = vec![n(0, 0.5, 40)];
    let beats = song.channels[3].lanes[0].add_clip(0, 1.0);
    beats.visible_len_beats = 1e6;
    beats.events = vec![n(0, 32.0, 50)];
    let clip = song.channels[1].lanes[0].clips[0].clone();
    // Beats 1000 to 1016 at 32 points a beat: 4 ticks a pixel.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(ghost_notes(
            &song,
            1,
            Some(&clip),
            4,
            128_000.0..130_048.0,
            4.0,
        ));
    });
    let ghosts = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the ghost notes within 5 s");
    let at: Vec<(f64, u8)> = ghosts.iter().map(|g| (g.pos, g.key)).collect();
    let expected: Vec<(f64, u8)> = (1000..1016).map(|b| (f64::from(b) * 128.0, 50)).collect();
    assert_eq!(at, expected);
}

#[test]
fn ghost_notes_are_other_channels_notes_inside_the_clip_window() {
    let song = ghost_song();
    let clip = &song.channels[1].lanes[0].clips[0];
    let ghosts = ghost_notes(&song, 1, Some(clip), 4, ALL, 0.0);
    // Channel 2's note at song tick 1124 is content tick 100 of the edited clip.
    assert_eq!(
        ghosts,
        [Ghost {
            pos: 100.0,
            len: 16.0,
            key: 41,
            hue: song.channels[2].hue
        }]
    );
}

#[test]
fn ghost_notes_follow_the_clip_offset() {
    let mut song = ghost_song();
    song.channels[1].lanes[0].clips[0].offset_beats = 1.0;
    let clip = &song.channels[1].lanes[0].clips[0];
    let ghosts = ghost_notes(&song, 1, Some(clip), 4, ALL, 0.0);
    // The clip starts playing at content tick 128: song 1124 is content 228.
    assert_eq!(ghosts.iter().map(|g| g.pos).collect::<Vec<_>>(), [228.0]);
}

#[test]
fn without_a_clip_ghosts_come_from_the_first_bar() {
    let mut song = ghost_song();
    song.channels[2].lanes[0].clips[0].start_tick = 0;
    let ghosts = ghost_notes(&song, 1, None, 4, ALL, 0.0);
    let keys: Vec<u8> = ghosts.iter().map(|g| g.key).collect();
    assert_eq!(keys, [40, 41]);
}

#[test]
fn the_playhead_maps_into_the_clip_content() {
    let song = ghost_song();
    let clip = &song.channels[1].lanes[0].clips[0];
    assert_eq!(content_tick(Some(clip), 1000), None);
    assert_eq!(content_tick(Some(clip), 1024), Some(0.0));
    assert_eq!(content_tick(Some(clip), 1100), Some(76.0));
    // Past the visible length (one 4-beat loop).
    assert_eq!(content_tick(Some(clip), 1024 + 512), None);
    let mut looped = clip.clone();
    looped.visible_len_beats = 8.0;
    looped.offset_beats = 1.0;
    assert_eq!(content_tick(Some(&looped), 1024 + 400), Some(16.0));
    assert_eq!(content_tick(None, 0), None);
    // A loop of half a tick is no loop (`Clip::loop_ticks`): the clip plays nothing, and has no ghosts' window.
    let mut half = clip.clone();
    half.loop_len_beats = 0.5 / f64::from(PPQ);
    half.visible_len_beats = 4.0;
    assert_eq!(content_tick(Some(&half), 1024), None);
}

#[test]
fn notes_above_key_127_are_left_alone() {
    // A file can hold any key byte; only 0..=127 are notes the roll can show or edit.
    assert_eq!(note(&n(0, 32.0, 200)), None);
    assert_eq!(G.note_rect(&n(0, 32.0, 200)), None);
    let orig = [n(0, 32.0, 0), n(32, 32.0, 200)];
    let both = [true, true];
    // The group clamp only sees key 0: down is blocked, up moves it; key 200 never changes.
    assert_eq!(
        spans(&shift_notes(&orig, &both, 0.0, -5)),
        [(0, 32.0, 0), (32, 32.0, 200)]
    );
    assert_eq!(
        spans(&shift_notes(&orig, &both, 0.0, 5)),
        [(0, 32.0, 5), (32, 32.0, 200)]
    );
    assert_eq!(
        spans(&stretch_notes(&orig, &both, 32.0, 1.0)),
        [(0, 64.0, 0), (32, 32.0, 200)]
    );
    let mut events = orig.to_vec();
    let mut sel = vec![true, true];
    double_selected(&mut events, &mut sel, 12);
    assert_eq!(
        spans(&events),
        [(0, 32.0, 0), (0, 32.0, 12), (32, 32.0, 200)]
    );
    // Nothing to hit at key 200's would-be row, and no panic finding the key 0 note.
    let low = G.y(0) + 8.0;
    assert_eq!(hit_test(&G, &orig, pos2(110.0, low)), Some(Hit::Body(0)));
}

// ---- note menu: scale, slide, transpose ----

/// `n` with the slide flag on.
fn slid(pos: u32, len: f64, key: u8) -> Event {
    Event::Note {
        pos,
        len_beats: len / f64::from(PPQ),
        key,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: true,
    }
}

fn slides(events: &[Event]) -> Vec<bool> {
    events
        .iter()
        .map(|e| matches!(e, Event::Note { slide: true, .. }))
        .collect()
}

/// The stored key-lock id of the picker scale `name`.
fn stored(name: &str) -> i32 {
    use crate::song::scales::{SCALES, scale_to_stored};
    scale_to_stored(SCALES.iter().position(|s| *s == name).unwrap())
}

const MAJOR: u16 = 0b1010_1101_0101;

#[test]
fn the_project_scale_is_the_songs_key_lock() {
    // A new song locks nothing: Chromatic on C.
    assert_eq!(project_scale(&Song::default()), (0, 0xFFF));
    let song = Song {
        key_root: 2,
        key_scale: stored("Major"),
        ..Song::default()
    };
    assert_eq!(project_scale(&song), (2, MAJOR));
    // A stored id the picker lacks reads as Chromatic, an out-of-range root as C (as the keyboard settings show them).
    let odd = Song {
        key_root: 40,
        key_scale: stored("Major") + 1000,
        ..Song::default()
    };
    assert_eq!(project_scale(&odd), (0, 0xFFF));
}

#[test]
fn snap_to_scale_moves_keys_to_the_nearest_scale_tone_the_lower_on_a_tie() {
    // C major: C# and D# sit between two tones (down), F# too (F, not G); in-scale keys stay.
    let keys = [60, 61, 63, 64, 66, 70];
    let mut events: Vec<Event> = keys.iter().map(|&k| n(0, 32.0, k)).collect();
    let all = vec![true; events.len()];
    snap_to_scale(&mut events, &all, 0, MAJOR);
    let snapped: Vec<u8> = spans(&events).iter().map(|s| s.2).collect();
    assert_eq!(snapped, [60, 60, 62, 64, 65, 69]);
    // D major (root 2): F (65) and C (60) are a semitone under F# and C#, a semitone over E and B: down.
    assert_eq!(snap_key(65, 2, MAJOR), 64);
    assert_eq!(snap_key(60, 2, MAJOR), 59);
    assert_eq!(snap_key(66, 2, MAJOR), 66);
    // Chromatic changes nothing; the key range bounds the search.
    assert_eq!(snap_key(61, 0, 0xFFF), 61);
    assert_eq!(
        snap_key(0, 2, MAJOR),
        1,
        "C is not in D major and nothing lies below 0"
    );
    assert_eq!(
        snap_key(127, 0, 0b0100_0000_0000),
        121,
        "only C#: down from G, as 133 is out"
    );
}

#[test]
fn snap_to_scale_changes_only_the_flagged_notes() {
    let mut events = vec![n(0, 32.0, 61), n(32, 32.0, 61), n(64, 32.0, 200)];
    snap_to_scale(&mut events, &[false, true, true], 0, MAJOR);
    assert_eq!(
        spans(&events),
        [(0, 32.0, 61), (32, 32.0, 60), (64, 32.0, 200)]
    );
}

#[test]
fn the_targets_are_the_selection_or_else_every_note() {
    let events = [n(0, 32.0, 60), n(32, 32.0, 200), n(64, 32.0, 62)];
    assert_eq!(
        targets(&events, &[false, false, true]),
        [false, false, true]
    );
    assert_eq!(targets(&events, &[false; 3]), [true, false, true]);
    assert_eq!(targets(&events, &[]), [true, false, true]);
}

#[test]
fn toggle_slide_flips_each_flagged_note() {
    let mut events = vec![n(0, 32.0, 60), slid(32, 32.0, 62), n(64, 32.0, 64)];
    toggle_slide(&mut events, &[true, true, false]);
    assert_eq!(slides(&events), [true, false, false]);
    toggle_slide(&mut events, &[true, true, false]);
    assert_eq!(slides(&events), [false, true, false]);
    // Keys above 127 are left alone, as every edit leaves them.
    let mut odd = vec![n(0, 32.0, 200)];
    toggle_slide(&mut odd, &[true]);
    assert_eq!(slides(&odd), [false]);
}

#[test]
fn new_notes_do_not_slide_and_edits_keep_the_flag() {
    let mut events = Vec::new();
    let mut sel = Vec::new();
    add_note(&mut events, &mut sel, 0.0, 32.0, 60);
    assert_eq!(slides(&events), [false]);
    let orig = [slid(0, 32.0, 60)];
    assert_eq!(slides(&shift_notes(&orig, &[true], 32.0, 2)), [true]);
    assert_eq!(slides(&stretch_notes(&orig, &[true], 32.0, 1.0)), [true]);
    let (mut events, mut sel) = (orig.to_vec(), vec![true]);
    duplicate_selected(&mut events, &mut sel, 32.0);
    assert_eq!(slides(&events), [true, true], "a copy keeps the flag");
    double_selected(&mut events, &mut sel, 12);
    assert_eq!(slides(&events), [true, true, true]);
}

#[test]
fn a_copy_keeps_velocity_and_pan() {
    let loud = Event::Note {
        pos: 0,
        len_beats: 0.25,
        key: 60,
        velocity: 255,
        pan: 0,
        slide: false,
    };
    let (mut events, mut sel) = (vec![loud], vec![true]);
    duplicate_selected(&mut events, &mut sel, 32.0);
    assert!(matches!(
        events[1],
        Event::Note {
            pos: 32,
            velocity: 255,
            pan: 0,
            ..
        }
    ));
}

#[test]
fn transpose_offsets_read_signed_above_zero_and_plain_otherwise() {
    assert_eq!(offset_label(3), "+3");
    assert_eq!(offset_label(0), "0");
    assert_eq!(offset_label(-12), "-12");
}

#[test]
fn the_transpose_range_keeps_the_whole_group_inside_the_keys() {
    let events = [n(0, 32.0, 10), n(32, 32.0, 100), n(64, 32.0, 200)];
    assert_eq!(transpose_range(&events, &[true, true, true]), (-10, 27));
    assert_eq!(transpose_range(&events, &[false, true, false]), (-100, 27));
    assert_eq!(transpose_range(&events, &[false, false, true]), (0, 0));
}

#[test]
fn a_scale_guess_needs_three_different_pitch_classes() {
    let octaves = [
        n(0, 32.0, 48),
        n(0, 32.0, 60),
        n(32, 32.0, 72),
        n(64, 32.0, 64),
    ];
    assert_eq!(guess_scales(&octaves, &[true; 4]), None);
    // Only the flagged notes count.
    let triad = [n(0, 32.0, 60), n(0, 32.0, 64), n(0, 32.0, 67)];
    assert_eq!(guess_scales(&triad, &[true, true, false]), None);
    assert!(guess_scales(&triad, &[true; 3]).is_some());
}

#[test]
fn scale_guesses_rank_fewest_accidentals_then_most_root_notes_then_picker_order() {
    // C E G C: C major fits with two root notes; Chromatic is never a candidate.
    let notes = [
        n(0, 32.0, 60),
        n(0, 32.0, 64),
        n(0, 32.0, 67),
        n(128, 32.0, 72),
    ];
    let guesses = guess_scales(&notes, &[true; 4]).unwrap();
    assert_eq!(guesses.len(), 32 * 12);
    let labels: Vec<String> = guesses
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, g)| g.label(i + 1))
        .collect();
    assert_eq!(
        labels,
        [
            "#1 C Major (All in scale, 2 root notes)",
            "#2 C Major Bebop (All in scale, 2 root notes)",
            "#3 C Major Pentatonic (All in scale, 2 root notes)",
        ]
    );
    let last = guesses.last().unwrap();
    assert!(last.accidentals > 0);
    assert_eq!(last.accidentals + last.in_scale, 4);
    // All twelve pitch classes: no exact match; the eight-tone scales come first, Major Bebop on C leading.
    let all: Vec<Event> = (60..72).map(|k| n(0, 32.0, k)).collect();
    let guesses = guess_scales(&all, &[true; 12]).unwrap();
    assert_eq!(
        guesses[0].label(1),
        "#1 C Major Bebop (4 accidentals, 8 in scale, 1 root notes)"
    );
    assert!(guesses.iter().all(|g| g.accidentals >= 4));
    // Root names are the picker's sharps.
    let sharp = [
        n(0, 32.0, 61),
        n(0, 32.0, 65),
        n(0, 32.0, 68),
        n(0, 32.0, 61),
    ];
    assert!(
        guess_scales(&sharp, &[true; 4]).unwrap()[0]
            .label(1)
            .starts_with("#1 C# Major ")
    );
}

/// A note at `pos` with velocity `velocity`.
fn loud(pos: u32, key: u8, velocity: u8) -> Event {
    match n(pos, 32.0, key) {
        Event::Note {
            len_beats,
            pan,
            slide,
            ..
        } => Event::Note {
            pos,
            len_beats,
            key,
            velocity,
            pan,
            slide,
        },
        other => other,
    }
}

/// `(pos, velocity)` of every note.
fn timing(events: &[Event]) -> Vec<(u32, u8)> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::Note { pos, velocity, .. } => Some((pos, velocity)),
            _ => None,
        })
        .collect()
}

/// Humanizes `events` (all selected) with the defaults, which are the plain `Humanize`, and every normal draw `z`.
fn humanized(events: &[Event], z: f64) -> Vec<Event> {
    humanize_with(
        events,
        &vec![true; events.len()],
        &Humanize::default(),
        || z,
    )
}

#[test]
fn humanize_without_deviation_changes_nothing() {
    let notes = [loud(0, 60, 204), loud(64, 62, 1), slid(128, 32.0, 64)];
    assert_eq!(humanized(&notes, 0.0), notes);
}

#[test]
fn humanize_moves_starts_by_a_fortieth_of_a_beat_per_sigma_truncated_to_ticks() {
    // 0.025 beat = 3.2 ticks a standard deviation; 0.05 of full velocity = 12.75 a standard deviation.
    let note = [loud(64, 60, 204)];
    assert_eq!(timing(&humanized(&note, 0.4)), [(65, 209)]); // +1.28 ticks, +5.1
    assert_eq!(timing(&humanized(&note, 0.1)), [(64, 205)]); // +0.32, +1.275
    assert_eq!(timing(&humanized(&note, -0.4)), [(62, 198)]); // 62.72, 198.9
    // Length, key, pan and slide stay.
    let slide = [slid(64, 48.0, 70)];
    assert_eq!(spans(&humanized(&slide, 0.4)), [(65, 48.0, 70)]);
    assert_eq!(slides(&humanized(&slide, 0.4)), [true]);
}

#[test]
fn humanize_keeps_starts_at_or_after_zero_velocity_in_1_to_255_and_lengths_at_least_a_tick() {
    let notes = [loud(10, 60, 204), loud(500, 62, 3)];
    assert_eq!(timing(&humanized(&notes, -1000.0)), [(0, 1), (0, 1)]);
    assert_eq!(timing(&humanized(&notes, 100.0)), [(330, 255), (820, 255)]);
    let wide = Humanize {
        length: 0.5,
        ..Humanize::default()
    };
    let short = humanize_with(&notes, &[true; 2], &wide, || -1000.0);
    assert_eq!(spans(&short), [(0, 1.0, 60), (0, 1.0, 62)]);
}

#[test]
fn humanize_changes_only_the_selected_notes_unless_it_applies_to_all() {
    let events = [loud(64, 60, 100), loud(128, 62, 100), loud(192, 200, 100)];
    // Each note draws its start, velocity and length deviations, in that order.
    let draws = || {
        let mut z = [1.0, -1.0, 0.0].into_iter().cycle();
        move || z.next().unwrap()
    };
    let sel = [false, true, true];
    let selected = humanize_with(&events, &sel, &Humanize::default(), draws());
    // Start +3.2 → +3 ticks, velocity −12.75 → 87; key 200 is not a note the roll edits.
    assert_eq!(timing(&selected), [(64, 100), (131, 87), (192, 100)]);
    let all = Humanize {
        all: true,
        ..Humanize::default()
    };
    let every = humanize_with(&events, &sel, &all, draws());
    assert_eq!(timing(&every), [(67, 87), (131, 87), (192, 100)]);
}

#[test]
fn humanize_repeats_by_seed_and_its_defaults_spread_as_the_specs() {
    let notes = vec![loud(1000, 60, 128); 4000];
    let sel = vec![true; notes.len()];
    let s = Humanize::default();
    let a = humanize(&notes, &sel, &s, 7);
    assert_eq!(
        a,
        humanize(&notes, &sel, &s, 7),
        "the same seed, the same notes"
    );
    assert_ne!(a, humanize(&notes, &sel, &s, 8));
    // N(0, 3.2 ticks) and N(0, 12.75), each floored to a whole step: means −0.5.
    let stats = |x: Vec<f64>| {
        let mean = x.iter().sum::<f64>() / x.len() as f64;
        let var = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / x.len() as f64;
        (mean, var.sqrt())
    };
    let (dt, dv): (Vec<f64>, Vec<f64>) = (timing(&a).into_iter())
        .map(|(p, v)| (f64::from(p) - 1000.0, f64::from(v) - 128.0))
        .unzip();
    let (mean, sd) = stats(dt);
    assert!(
        (mean + 0.5).abs() < 0.2 && (sd - 3.2).abs() < 0.15,
        "{mean} {sd}"
    );
    let (mean, sd) = stats(dv);
    assert!(
        (mean + 0.5).abs() < 0.8 && (sd - 12.75).abs() < 0.6,
        "{mean} {sd}"
    );
    assert!(spans(&a).iter().all(|n| n.1 == 32.0 && n.2 == 60));
    // Pressing `Apply` on an open panel does what `Humanize` did.
    assert_eq!(super::humanize::Panel::new(0, true).settings(), s);
}

#[test]
fn humanize_modulators_shorten_loud_and_high_notes_at_positive_amounts() {
    let still = Humanize {
        timing: 0.0,
        velocity: 0.0,
        all: true,
        ..Humanize::default()
    };
    let lens = |s: Humanize, notes: &[Event]| -> Vec<f64> {
        spans(&humanize(notes, &[], &s, 1))
            .iter()
            .map(|n| n.1)
            .collect()
    };
    // Velocity 1.0 and 0.2 on one key; keys 72 and 60 at one velocity.
    let (by_velocity, by_pitch) = (
        [loud(0, 60, 255), loud(0, 60, 51)],
        [loud(0, 72, 128), loud(0, 60, 128)],
    );
    for (amount, louder_shorter) in [(1.0, true), (-1.0, false)] {
        let v = lens(
            Humanize {
                vel_to_len: amount,
                ..still
            },
            &by_velocity,
        );
        assert_eq!(v[0] < v[1], louder_shorter, "{amount}: {v:?}");
        let p = lens(
            Humanize {
                pitch_to_len: amount,
                ..still
            },
            &by_pitch,
        );
        assert_eq!(p[0] < p[1], louder_shorter, "{amount}: {p:?}");
    }
    assert_eq!(lens(still, &by_velocity), [32.0, 32.0]);
}

#[test]
fn humanize_base_adjustments_come_before_the_deviations() {
    let base = Humanize {
        vel_offset: 0.1,
        len_scale: 2.0,
        time_offset: 4.0,
        ..Humanize::default()
    };
    // Velocity +25.5 (truncated), length ×2, start +4 ticks.
    let out = humanize_with(&[loud(64, 60, 204)], &[true], &base, || 0.0);
    assert_eq!(timing(&out), [(68, 229)]);
    assert_eq!(spans(&out), [(68, 64.0, 60)]);
    // The offset velocity is kept to 0..=255 before its deviation: 255 − 12.75, not 331.5 − 12.75.
    let up = Humanize {
        vel_offset: 0.5,
        ..base
    };
    let out = humanize_with(&[loud(64, 60, 204)], &[true], &up, || -1.0);
    assert_eq!(timing(&out), [(64, 242)]);
}

#[test]
fn the_normal_generator_has_mean_0_and_deviation_1_and_differs_by_seed() {
    let mut draw = normal(12345);
    let z: Vec<f64> = (0..20_000).map(|_| draw()).collect();
    let mean = z.iter().sum::<f64>() / z.len() as f64;
    let sd = (z.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / z.len() as f64).sqrt();
    assert!(mean.abs() < 0.03, "{mean}");
    assert!((sd - 1.0).abs() < 0.03, "{sd}");
    let (mut a, mut b) = (normal(1), normal(2));
    assert_ne!((a(), a()), (b(), b()));
    assert!(z.iter().all(|x| x.is_finite()));
}

// ---- the component, offscreen ----

mod component {
    use eframe::egui::{
        self, Context, Event as In, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rangef,
        RawInput, Rect, Vec2, pos2, vec2,
    };

    use eframe::egui::{TouchDeviceId, TouchId, TouchPhase};

    use super::{U, n, spans};
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
    use crate::song::scales::{SCALES, scale_to_stored};
    use crate::song::{ChannelKind, Event, PPQ, Song};
    use crate::theme::theme_by_id;
    use crate::ui::UiCtx;
    use crate::ui::clips::{Selection, place_row};
    use crate::ui::piano_roll::notes::{Geom, SnapMode};
    use crate::ui::piano_roll::{BEAT_U, PianoRoll, PianoRollAction, Remembered, RollOptions};
    use crate::ui::timeline;

    /// An editor area 1170 × 349 points at U = 32 (ppp 1): the playlist's ruler and track bands before the 5 U panel.
    const RECT: Rect = Rect {
        min: Pos2 { x: 0.0, y: 0.0 },
        max: Pos2 {
            x: 1170.0,
            y: 349.0,
        },
    };
    /// Scrolled so that key 72's row is the top one; the grid starts at (50, [`TOP`]) (the 1.5 U key column and t), a
    /// beat is 64 points, a row 16.
    const SCROLL_Y: f32 = 880.0;
    /// The grid's top: the ruler's height at U = 32.
    const TOP: f32 = 52.0;
    const EDITED: PianoRollAction = PianoRollAction::SongEdited { final_edit: false };
    const DONE: PianoRollAction = PianoRollAction::SongEdited { final_edit: true };
    /// A tap's new note: final, and no re-sync.
    const QUIET: PianoRollAction = PianoRollAction::QuietEdit;

    /// `x` to a thousandth of a point, for a play box's playhead x (its centre less half its width).
    fn r3(x: f32) -> f32 {
        (x * 1000.0).round() / 1000.0
    }

    /// x of `tick` and the y of the middle of `key`'s row, in the default view.
    fn at(tick: f64, key: u8) -> Pos2 {
        let g = Geom {
            origin: pos2(50.0, TOP - SCROLL_Y),
            beat_w: 64.0,
            row_h: 16.0,
        };
        pos2(g.x(tick), g.y(key) + 8.0)
    }

    struct Rig {
        ctx: Context,
        roll: PianoRoll,
        song: Song,
        opts: RollOptions,
        time: f64,
        /// What the last frame returned, and every action since the rig was made, but the sounds, which `sounds`
        /// collects as (key, velocity or `None` for an off).
        last: Vec<PianoRollAction>,
        all: Vec<PianoRollAction>,
        sounds: Vec<(u8, Option<f32>)>,
        texts: Vec<(String, Rect)>,
        /// The filled rects of the last frame.
        rects: Vec<Rect>,
        /// [`Self::rects`] with their fills, in paint order.
        painted: Vec<(Rect, egui::Color32)>,
        /// The playhead x the play box showed last frame, to a thousandth of a point ([`r3`]), and whether it showed the
        /// REC icon.
        handle: Option<f32>,
        armed: bool,
        /// Held through every frame.
        modifiers: Modifiers,
        /// Every shape of the last frame in paint order, groups flattened.
        shapes: Vec<egui::Shape>,
        /// What shows of each filled rect of the last frame (the rect inside its clip), groups flattened.
        visible: Vec<(Rect, egui::Color32)>,
    }

    impl Rig {
        /// Channel 1 (an instrument) with a one-bar clip at song tick 0 holding `notes`.
        fn new(notes: Vec<Event>) -> Self {
            let mut song = Song::default();
            song.add_channel(ChannelKind::Instrument);
            song.channels[1].lanes[0].add_clip(0, 4.0).events = notes;
            Self::with_song(song)
        }

        fn with_song(song: Song) -> Self {
            let roll = PianoRoll::for_channel(&song, 1);
            let mut rig = Self {
                ctx: Context::default(),
                roll,
                song,
                opts: RollOptions {
                    ghost_notes: true,
                    ..Default::default()
                },
                time: 0.0,
                last: Vec::new(),
                all: Vec::new(),
                sounds: Vec::new(),
                texts: Vec::new(),
                rects: Vec::new(),
                painted: Vec::new(),
                handle: None,
                armed: false,
                modifiers: Modifiers::NONE,
                shapes: Vec::new(),
                visible: Vec::new(),
            };
            rig.frame(vec![]);
            rig.roll.scroll = Some(vec2(0.0, SCROLL_Y));
            rig.frame(vec![]);
            rig
        }

        fn frame_after(&mut self, dt: f64, events: Vec<In>) {
            self.time += dt;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0))),
                time: Some(self.time),
                modifiers: self.modifiers,
                events,
                ..Default::default()
            };
            let (theme, m) = (
                theme_by_id(0),
                Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE),
            );
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            let (roll, song, opts) = (&mut self.roll, &mut self.song, self.opts);
            let mut last = Vec::new();
            let output = self.ctx.run(raw, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| last = roll.show(ui, RECT, &cx, song, &opts));
            });
            for action in std::mem::take(&mut last) {
                match action {
                    PianoRollAction::Note { key, velocity } => self.sounds.push((key, velocity)),
                    other => last.push(other),
                }
            }
            self.all.extend(last.iter().copied());
            self.last = last;
            self.handle = timeline::testing::play_box_x(&output.shapes, U).map(r3);
            self.armed = timeline::testing::play_box_armed(&output.shapes);
            self.texts = output
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some((
                        t.galley.text().to_owned(),
                        t.galley.rect.translate(t.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect();
            self.shapes.clear();
            self.visible.clear();
            for c in &output.shapes {
                let group = match &c.shape {
                    egui::Shape::Vec(v) => v.clone(),
                    shape => vec![shape.clone()],
                };
                for shape in group {
                    if let egui::Shape::Rect(r) = &shape {
                        self.visible.push((r.rect.intersect(c.clip_rect), r.fill));
                    }
                    self.shapes.push(shape);
                }
            }
            self.rects = (self.shapes.iter())
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) => Some(r.rect),
                    _ => None,
                })
                .collect();
            self.painted = crate::ui::dialog::painted_rects(output.shapes.iter().map(|c| &c.shape));
        }

        fn frame(&mut self, events: Vec<In>) {
            self.frame_after(1.0 / 60.0, events);
        }

        fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
            self.frame(vec![In::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }

        fn tap(&mut self, pos: Pos2) {
            self.frame(vec![In::PointerMoved(pos)]);
            self.button(pos, PointerButton::Primary, true);
            self.button(pos, PointerButton::Primary, false);
        }

        /// The playhead at beat 2: its play box, whose touch area at beat 0 covers part of the Snap button and the
        /// ruler's first beat and more (and takes their touches), clear of them.
        fn knob_away(&mut self) {
            self.opts.playhead = 256;
            self.frame(vec![]);
        }

        /// Press at `from`, move to `to` in two steps, release there. Returns the actions of the move frames.
        fn drag(&mut self, from: Pos2, to: Pos2) -> Vec<PianoRollAction> {
            self.frame(vec![In::PointerMoved(from)]);
            self.button(from, PointerButton::Primary, true);
            let mut during = Vec::new();
            for p in [from + (to - from) / 2.0, to] {
                self.frame(vec![In::PointerMoved(p)]);
                during.extend(self.last.iter().copied());
            }
            self.button(to, PointerButton::Primary, false);
            during
        }

        /// Press at `pos` and hold it for a second (a long press); still down afterwards.
        fn hold(&mut self, pos: Pos2) {
            self.frame(vec![In::PointerMoved(pos)]);
            self.button(pos, PointerButton::Primary, true);
            self.frame_after(1.0, vec![]);
        }

        fn release(&mut self, pos: Pos2) {
            self.button(pos, PointerButton::Primary, false);
        }

        /// A finger: the touch event plus the pointer events an integration sends with it.
        fn touch(&mut self, dt: f64, phase: TouchPhase, pos: Pos2) {
            let touch = In::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(1),
                phase,
                pos,
                force: None,
            };
            let pointer = match phase {
                TouchPhase::Start => vec![
                    In::PointerMoved(pos),
                    In::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                ],
                TouchPhase::Move => vec![In::PointerMoved(pos)],
                TouchPhase::End | TouchPhase::Cancel => vec![
                    In::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                    In::PointerGone,
                ],
            };
            self.frame_after(dt, [vec![touch], pointer].concat());
        }

        /// A second finger: its touch event only, as an integration sends for every finger but the first.
        fn second(&mut self, phase: TouchPhase, pos: Pos2) {
            self.frame(vec![In::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(2),
                phase,
                pos,
                force: None,
            }]);
        }

        /// A finger held still on `pos` for a second (egui's long touch); still down afterwards.
        fn touch_hold(&mut self, pos: Pos2) {
            self.touch(1.0 / 60.0, TouchPhase::Start, pos);
            self.touch(1.0, TouchPhase::Move, pos);
        }

        fn events(&self) -> &[Event] {
            &self.song.channels[1].lanes[0].clips[0].events
        }

        /// Centre of the text `label` drawn last frame.
        fn text(&self, label: &str) -> Option<Pos2> {
            self.texts
                .iter()
                .find(|(t, _)| t == label)
                .map(|(_, r)| r.center())
        }

        /// Selects `which` and shows the floating row where `Select All` anchors it, the grid's centre.
        fn select(&mut self, which: Vec<bool>) {
            self.roll.selected = which;
            self.frame(vec![]);
            self.roll.row.select_all();
            self.frame(vec![]);
        }
    }

    /// (velocity, pan, slide) of every note.
    fn looks(events: &[Event]) -> Vec<(u8, u8, bool)> {
        (events.iter())
            .filter_map(|e| match *e {
                Event::Note {
                    velocity,
                    pan,
                    slide,
                    ..
                } => Some((velocity, pan, slide)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn at_beat_0_the_lines_left_half_shows_in_the_gap_after_the_key_column() {
        // The playhead line's z-order: the key column covers only what lies left of its right edge (x 48); the grid
        // starts t = 2 points on, at the line's centre (x 50).
        let mut rig = Rig::new(vec![]);
        rig.frame(vec![]);
        let c = theme_by_id(0).colors;
        let grid = (rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("grid")))
        .unwrap()
        .rect;
        // What shows of it reaches both sides of x 50, over the grid's height.
        let both_sides = |fill: egui::Color32| {
            (rig.visible.iter()).any(|&(r, f)| {
                f == fill && r.y_range() == grid.y_range() && r.left() < 50.0 && r.right() > 50.0
            })
        };
        assert!(both_sides(c.text.to_color32()), "the line, both halves");
        let shadow = c.background.to_color32().gamma_multiply(0.3);
        assert!(both_sides(shadow), "the shadow, both halves");
    }

    #[test]
    fn the_playhead_line_lies_over_the_notes_and_under_their_handles() {
        // The playhead line's z-order: the notes, then the line, then the grid's children.
        let mut rig = Rig::new(vec![n(0, 512.0, 70)]);
        rig.roll.selected = vec![true];
        rig.opts.playhead = 128;
        rig.frame(vec![]);
        let theme = theme_by_id(0);
        let text = theme.colors.text.to_color32();
        let x = at(128.0, 70).x;
        use crate::ui::timeline::testing::line_x;
        let position = |f: &dyn Fn(&egui::Shape) -> bool| rig.shapes.iter().position(f);
        let note = position(
            &|s| matches!(s, egui::Shape::Rect(r) if r.rect.contains(at(64.0, 70)) && r.fill != text),
        );
        let line = position(
            &|s| matches!(s, egui::Shape::Rect(r) if r.fill == text && r.rect.top() >= TOP && r.rect.width() < U && line_x(r.rect, 1.0) == x),
        );
        let handle = position(
            &|s| matches!(s, egui::Shape::Rect(r) if r.fill == text && r.corner_radius != egui::CornerRadius::ZERO),
        );
        let (note, line, handle) = (
            note.expect("the note"),
            line.expect("the line"),
            handle.expect("its handle"),
        );
        assert!(note < line && line < handle, "{note} {line} {handle}");
    }

    #[test]
    fn while_rec_is_armed_the_play_box_shows_the_record_icon() {
        // The play box: icon 5 while REC is armed, else 3.
        let mut rig = Rig::new(vec![]);
        assert!(rig.handle.is_some() && !rig.armed);
        rig.opts.armed = true;
        rig.frame(vec![]);
        assert!(rig.armed);
    }

    #[test]
    fn the_grid_and_the_time_ruler_start_t_after_the_key_column() {
        // A t gap (2 points at U = 32) after the 1.5 U column, as in the mini mixer's ruler row.
        let mut rig = Rig::new(vec![]);
        rig.frame(vec![]);
        let grid = (rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("grid")))
        .unwrap()
        .rect;
        assert_eq!(grid.left(), 48.0 + 2.0);
        // Tick 0's ruler tap lands on the grid's edge.
        assert_eq!(at(0.0, 60).x, grid.left());
    }

    #[test]
    fn a_tap_on_empty_grid_adds_an_unselected_note_in_the_tapped_cell_and_sounds_it_for_150_ms() {
        // Start = the tapped cell's start, length = max(L, unit/8) with the remembered note (L = 1 beat, V 204, P 127,
        // no slide at first).
        let mut rig = Rig::new(vec![]);
        rig.tap(at(100.0, 70));
        // At 64 points a beat, visible grid lines are half a beat apart: 100 → 64.
        assert_eq!(spans(rig.events()), [(64, 128.0, 70)]);
        assert_eq!(looks(rig.events()), [(204, 127, false)]);
        assert_eq!(rig.roll.selected, [false]);
        assert_eq!(rig.all, [QUIET]);
        assert_eq!(rig.sounds, [(70, Some(1.0))]);
        rig.frame_after(0.2, vec![]);
        assert_eq!(rig.sounds, [(70, Some(1.0)), (70, None)]);
        // The cell it hits, not the nearest line: 113 → 64.
        rig.tap(at(113.0, 66));
        assert!(spans(rig.events()).contains(&(64, 128.0, 66)));
        // The remembered note, its length never under an eighth of the unit.
        rig.roll.remembered = Remembered {
            len: 2.0,
            velocity: 90,
            pan: 30,
            slide: true,
        };
        rig.tap(at(300.0, 62));
        assert_eq!(spans(rig.events())[2], (256, 8.0, 62));
        assert_eq!(looks(rig.events())[2], (90, 30, true));
    }

    #[test]
    fn with_snap_off_the_note_starts_under_the_pointer_at_least_1_128_beat_long() {
        let mut rig = Rig::new(vec![]);
        rig.roll.snap.on = false;
        rig.roll.remembered.len = 0.5;
        rig.tap(at(100.0, 70));
        assert_eq!(spans(rig.events()), [(100, 1.0, 70)]);
        rig.roll.snap.on = true;
        rig.roll.snap.mode = SnapMode::Beat;
        rig.roll.remembered.len = 64.0;
        rig.tap(at(300.0, 66));
        assert_eq!(spans(rig.events())[1], (256, 64.0, 66));
    }

    #[test]
    fn a_new_note_snaps_to_the_project_scale() {
        let mut rig = Rig::new(vec![]);
        let major = SCALES.iter().position(|s| *s == "Major").unwrap();
        (rig.song.key_root, rig.song.key_scale) = (0, scale_to_stored(major));
        // C# between C and D: the lower one.
        rig.tap(at(100.0, 61));
        assert_eq!(spans(rig.events()), [(64, 128.0, 60)]);
        assert_eq!(rig.sounds, [(60, Some(1.0))]);
    }

    #[test]
    fn with_a_selection_showing_a_tap_on_empty_grid_only_deselects() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.tap(at(140.0, 70));
        assert_eq!(rig.roll.selected, [true]);
        rig.tap(at(400.0, 66));
        assert_eq!(rig.roll.selected, [false]);
        assert_eq!(rig.events().len(), 1);
        rig.tap(at(400.0, 66));
        assert_eq!(rig.events().len(), 2);
    }

    #[test]
    fn an_empty_tap_after_a_move_creates_a_note_without_a_deselection_tap() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.drag(at(140.0, 70), at(268.0, 70));
        assert!(rig.roll.quiet);
        assert_eq!(rig.roll.selected, [true]);
        assert!(!rig.roll.row.showing());
        rig.tap(at(400.0, 66));
        assert_eq!(spans(rig.events()), [(256, 64.0, 70), (384, 64.0, 66)]);
        assert_eq!(rig.roll.selected, [false, false]);
    }

    #[test]
    fn a_hidden_menu_does_not_consume_empty_taps_inside_or_outside_a_selection_frame() {
        for tick in [200.0, 600.0] {
            let mut rig = Rig::new(vec![n(128, 64.0, 70), n(384, 64.0, 62)]);
            // Keyboard selection keeps the handles, with its floating menu hidden.
            rig.key(egui::Key::A, CMD);
            assert_eq!(rig.roll.selected, [true, true]);
            assert!(!rig.roll.row.showing());
            rig.tap(at(tick, 66));
            assert_eq!(rig.events().len(), 3, "tick {tick}");
            assert!(!rig.roll.selected.contains(&true));
        }
    }

    #[test]
    fn new_notes_follow_visible_grid_cells_across_zoom_and_triplets() {
        for (beat_u, subdivision, tick, start) in [
            (2.0, 4, 128.0, 128),
            (1.0, 4, 113.0, 0),
            (4.0, 4, 113.0, 96),
            (2.0, 3, 80.0, 43),
        ] {
            let mut rig = Rig::new(vec![]);
            rig.roll.beat_u = beat_u;
            rig.song.time_sig.subdivision = subdivision;
            rig.frame(vec![]);
            let g = rig.roll.geom(GRID.min, 32.0);
            rig.tap(pos2(g.x(tick), g.y(66) + g.row_h / 2.0));
            assert_eq!(spans(rig.events()), [(start, 128.0, 66)]);
        }
    }

    #[test]
    fn adding_past_the_pattern_end_grows_the_loop_to_whole_bars() {
        let mut rig = Rig::new(vec![]);
        rig.tap(at(600.0, 70));
        assert_eq!(spans(rig.events()), [(576, 128.0, 70)]);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 8.0);
    }

    #[test]
    fn a_channel_without_a_clip_takes_no_note_and_gets_no_clip() {
        // The playlist's `+Clip` makes clips; the roll edits the one the app opens it on.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        let mut rig = Rig::with_song(song);
        assert_eq!(rig.roll.clip_id, None);
        rig.tap(at(40.0, 70));
        assert!(rig.song.channels[1].lanes[0].clips.is_empty());
        assert_eq!(rig.roll.clip_id, None);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_tap_on_a_note_selects_it_silently_and_a_second_tap_deselects_it_never_deleting() {
        // A second tap deselects; it opens no menu.
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        rig.tap(at(140.0, 70));
        assert_eq!(rig.roll.selected, [true, false]);
        for label in ["Copy", "Delete", "Snap", "More..."] {
            assert!(rig.text(label).is_some(), "no {label}");
        }
        rig.tap(at(140.0, 70));
        assert_eq!(rig.roll.selected, [false, false]);
        assert_eq!(rig.text("Copy"), None);
        // While a selection shows, a tap on another note only deselects too.
        rig.tap(at(140.0, 70));
        rig.tap(at(270.0, 60));
        assert_eq!(rig.roll.selected, [false, false]);
        assert_eq!(rig.events().len(), 2);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
        assert!(rig.sounds.is_empty(), "{:?}", rig.sounds);
    }

    #[test]
    fn a_touch_in_the_rows_margin_binds_the_note_but_a_tap_there_selects_nothing() {
        // Key 70's row is 16 points tall; the band reaches 0.2 row (3.2 points) into key 71's.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let margin = at(140.0, 70) - vec2(0.0, 8.0 + 2.0);
        rig.tap(margin);
        assert_eq!(rig.roll.selected, [false]);
        assert_eq!(rig.events().len(), 1, "no note added");
        // A drag from there moves the note a beat on.
        rig.drag(margin, margin + vec2(64.0, 0.0));
        assert_eq!(spans(rig.events()), [(256, 64.0, 70)]);
    }

    #[test]
    fn a_press_moved_0_4_u_and_lifted_within_0_3_s_is_a_tap() {
        // The calm zone is 0.5 U (16 points): 0.4 U is no drag.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let p = at(140.0, 70);
        let q = p + vec2(12.8, 0.0);
        rig.frame(vec![In::PointerMoved(p)]);
        rig.button(p, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(q)]);
        rig.button(q, PointerButton::Primary, false);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert_eq!(rig.roll.selected, [true]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    #[test]
    fn a_marquee_round_one_note_leaves_the_remembered_note() {
        // The remembered note's list has no marquee.
        let mut rig = Rig::new(vec![n(128, 96.0, 70)]);
        let from = at(400.0, 66);
        rig.hold(from);
        rig.frame(vec![In::PointerMoved(at(100.0, 71))]);
        rig.release(at(100.0, 71));
        assert_eq!(rig.roll.selected, [true]);
        assert_eq!(rig.roll.remembered, Remembered::default());
    }

    #[test]
    fn a_long_press_on_a_note_shows_the_selection_it_made_and_never_toggles() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        let p = at(140.0, 70);
        rig.tap(p);
        assert!(rig.text("Copy").is_some());
        // Held: the selection's row hides from the touch-down on; at the release it shows again, still selected.
        rig.hold(p);
        assert_eq!(rig.text("Copy"), None);
        rig.release(p);
        assert_eq!(rig.roll.selected, [true, false]);
        assert!(rig.text("Copy").is_some());
        // On the other note: it becomes the selection.
        let q = at(270.0, 60);
        rig.hold(q);
        rig.release(q);
        assert_eq!(rig.roll.selected, [false, true]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    #[test]
    fn a_touch_down_off_the_selection_drags_the_touched_note_alone_and_inside_its_frame_the_selection()
     {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 68), n(512, 64.0, 60)]);
        rig.tap(at(140.0, 70));
        // Off the selection's frame: the touched note becomes the selection and moves alone.
        let from = at(520.0, 60);
        rig.drag(from, from + vec2(64.0, 0.0));
        assert_eq!(spans(rig.events())[2], (640, 64.0, 60));
        assert_eq!(rig.roll.selected, [false, false, true]);
        // Two selected and showing: a touch on empty grid inside their frame (tick 160, key 69) moves both.
        (rig.roll.selected, rig.roll.quiet) = (vec![true, true, false], false);
        rig.frame(vec![]);
        let inside = at(160.0, 69);
        rig.drag(inside, inside + vec2(64.0, 0.0));
        assert_eq!(
            spans(rig.events()),
            [(256, 64.0, 70), (384, 64.0, 68), (640, 64.0, 60)]
        );
        assert_eq!(rig.roll.selected, [true, true, false]);
    }

    #[test]
    fn a_new_note_takes_the_last_lone_note_touched_moved_or_resized() {
        // The remembered note is set whenever one note alone becomes the selection, and after a lone note's move or
        // resize; `Set length` leaves it, `Toggle Slide` sets S.
        let mut rig = Rig::new(vec![n(128, 96.0, 70), n(512, 160.0, 66)]);
        // Tapped: its 96 ticks. (The second tap deselects; the third adds.)
        rig.tap(at(140.0, 70));
        rig.tap(at(140.0, 70));
        rig.tap(at(390.0, 58));
        assert!(
            spans(rig.events()).contains(&(384, 96.0, 58)),
            "{:?}",
            spans(rig.events())
        );
        // Resized by its handle (0.3 U past its end, x 384): the end 672 → 736.
        rig.tap(at(520.0, 66));
        let handle = pos2(384.0 + 9.6, at(0.0, 66).y);
        rig.drag(handle, handle + vec2(32.0, 0.0));
        assert!(spans(rig.events()).contains(&(512, 224.0, 66)));
        assert_eq!(rig.roll.remembered.len, 224.0);
        // `Set length` does not set L; `Toggle Slide` sets S.
        more(&mut rig, "Set length 1/16");
        assert_eq!(rig.roll.remembered.len, 224.0);
        escape(&mut rig);
        rig.roll.selected.fill(false);
        rig.tap(at(140.0, 70));
        more(&mut rig, "Toggle Slide");
        assert!(rig.roll.remembered.slide);
    }

    #[test]
    fn dragging_a_one_key_selection_sounds_each_key_it_reaches_until_the_release() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        // A row up, then two: each key change stops the old key and plays the new one at 0.7; the release stops it.
        let from = at(140.0, 70);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        assert!(rig.sounds.is_empty(), "touching is silent");
        rig.frame(vec![In::PointerMoved(from - vec2(0.0, 20.0))]);
        rig.frame(vec![In::PointerMoved(from - vec2(0.0, 32.0))]);
        rig.button(from - vec2(0.0, 32.0), PointerButton::Primary, false);
        assert_eq!(spans(rig.events())[0], (128, 64.0, 72));
        assert_eq!(
            rig.sounds,
            [(71, Some(0.7)), (71, None), (72, Some(0.7)), (72, None)]
        );
        // A selection two keys tall plays nothing.
        rig.sounds.clear();
        rig.roll.selected = vec![true, true];
        rig.frame(vec![]);
        let from = at(140.0, 72);
        rig.drag(from, from + vec2(0.0, -32.0));
        assert!(rig.sounds.is_empty(), "{:?}", rig.sounds);
    }

    #[test]
    fn dragging_a_note_moves_it_by_whole_snaps_and_reports_the_end() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        // 70 points right = 140 ticks: 268 rounds to 256. Two rows up.
        let from = at(140.0, 70);
        let during = rig.drag(from, from + vec2(70.0, -32.0));
        assert_eq!(spans(rig.events()), [(256, 64.0, 72)]);
        assert!(during.contains(&EDITED), "{during:?}");
        assert!(!during.contains(&DONE), "{during:?}");
        assert_eq!(rig.last, [DONE]);
    }

    #[test]
    fn a_drag_back_to_its_start_still_ends_with_one_final_edit() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let from = at(140.0, 70);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(from + vec2(64.0, 0.0))]);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, false);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        // The ongoing edit opened a gesture (the way back reports none: nothing to fit); the release closes it (a
        // final edit of an unchanged song records no step).
        assert_eq!(rig.all, [EDITED, DONE]);
    }

    #[test]
    fn the_end_capsule_puts_the_box_end_on_the_snapped_finger_and_every_length_changes_as_much() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 32.0, 66)]);
        rig.tap(at(140.0, 70));
        // The note ends at tick 192, its capsule 0.15 U on. No grab offset: the finger 39.6 points past the end is at
        // tick 271, which rounds to 256 on the 32-tick grid.
        let handle = at(192.0, 70) + vec2(9.6, 0.0);
        rig.drag(handle, handle + vec2(30.0, 0.0));
        assert_eq!(spans(rig.events()), [(128, 128.0, 70), (256, 32.0, 66)]);
        assert_eq!(rig.last, [DONE]);
        // Both: one capsule on the box [128, 288] (keys 70…66), centred on it. Its end 367 → 352: each note + 64.
        (rig.roll.selected, rig.roll.quiet) = (vec![true, true], false);
        rig.frame(vec![]);
        let handle = pos2(at(288.0, 70).x + 9.6, (at(0.0, 70).y + at(0.0, 66).y) / 2.0);
        rig.drag(handle, handle + vec2(30.0, 0.0));
        assert_eq!(spans(rig.events()), [(128, 192.0, 70), (256, 96.0, 66)]);
    }

    #[test]
    fn the_scale_bar_scales_the_selection_about_its_start_to_the_snapped_end_in_one_step() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 128.0, 66)]);
        // The box [128, 384]: its right edge at x 242, its bottom (key 66's row) 8 points under that row's centre.
        (rig.roll.selected, rig.roll.quiet) = (vec![true, true], false);
        rig.frame(vec![]);
        // The ↔ bar: 34 points wide, ending on the box's right edge, its top 3.84 points under the box.
        let bar = pos2(242.0 - 17.0, at(0.0, 66).y + 8.0 + 3.84 + 9.96);
        // The finger at tick 650: the box's end on 640, k = 512 / 256 = 2 about A = 128.
        rig.drag(bar, bar + vec2(150.0, 0.0));
        assert_eq!(spans(rig.events()), [(128, 128.0, 70), (384, 256.0, 66)]);
        assert_eq!(
            rig.all.iter().filter(|a| **a == DONE).count(),
            1,
            "{:?}",
            rig.all
        );
        assert_eq!(rig.roll.selected, [true, true]);
        // Far left of the start: the box's minimum width, 32/8 = 4 ticks, so k = 4 / 512.
        let bar = bar + vec2(128.0, 0.0);
        rig.drag(bar, bar - vec2(400.0, 0.0));
        assert_eq!(spans(rig.events()), [(128, 1.0, 70), (130, 2.0, 66)]);
    }

    #[test]
    fn a_huge_loop_scrolled_far_draws_without_overflow() {
        // A loop no song makes, but a file can hold: its step lines lie past u32 ticks.
        let mut rig = Rig::new(vec![]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 2e9;
        rig.roll.scroll = Some(vec2(1e11, SCROLL_Y));
        rig.frame(vec![]);
    }

    #[test]
    fn dragging_empty_space_scrolls() {
        let mut rig = Rig::new(vec![]);
        // Eight bars: wider than the grid.
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        let from = at(600.0, 64);
        rig.drag(from, from + vec2(-100.0, -40.0));
        // 1:1, then the release frame's fling step: 0.9 of the last move (50, 20).
        assert_eq!(rig.roll.scroll, Some(vec2(145.0, SCROLL_Y + 58.0)));
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_touch_on_empty_grid_scrolls_each_axis_past_its_calm_zone_one_to_one_and_flings() {
        // Per axis 0.5 U (16 points) still, then 1:1; a release after a move over 0.125 U flings, v ← 0.9·v a frame.
        let mut rig = Rig::new(vec![]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        let from = at(600.0, 64);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        // 10 points sideways and 20 up: only y has left its calm zone, and follows this move's 20 points.
        rig.frame(vec![In::PointerMoved(from + vec2(-10.0, -20.0))]);
        assert_eq!(rig.roll.scroll, Some(vec2(0.0, SCROLL_Y + 20.0)));
        // 30 more sideways: x follows those 30.
        let to = from + vec2(-40.0, -20.0);
        rig.frame(vec![In::PointerMoved(to)]);
        assert_eq!(rig.roll.scroll, Some(vec2(30.0, SCROLL_Y + 20.0)));
        // Let go: the time coasts 27 points in the release frame, then 24.3; the rows (no last move) stay.
        rig.button(to, PointerButton::Primary, false);
        for x in [57.0, 81.3] {
            let scroll = rig.roll.scroll.unwrap();
            assert!(
                (scroll - vec2(x, SCROLL_Y + 20.0)).length() < 1e-3,
                "{scroll:?}"
            );
            rig.frame(vec![]);
        }
        assert!(rig.events().is_empty() && rig.all.is_empty());
    }

    #[test]
    fn follow_playhead_keeps_the_folded_playhead_a_quarter_in_and_turns_the_one_finger_time_scroll_off()
     {
        // Follow playhead: the playhead folded into the clip's play range (clip-relative, looping); one-finger
        // horizontal scrolling off, vertical still on.
        let mut rig = Rig::new(vec![]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        (rig.opts.playing, rig.opts.continuous) = (true, true);
        rig.opts.playhead = 42 * u64::from(PPQ);
        rig.frame(vec![]);
        // 1120 points (after the key column and t) show 17.5 beats; beat 10 a quarter in.
        let left = |beat: f32| (beat - 1120.0 / 64.0 / 4.0) * 64.0;
        assert!((rig.roll.scroll.unwrap().x - left(10.0)).abs() < 1e-2);
        let from = at(600.0, 64);
        rig.drag(from, from + vec2(-200.0, -40.0));
        let scroll = rig.roll.scroll.unwrap();
        assert!((scroll.x - left(10.0)).abs() < 1e-2, "{scroll:?}");
        // 40 points, and the release frame's fling step (0.9 of the last move's 20).
        assert_eq!(scroll.y, SCROLL_Y + 58.0);
        // Not playing, or the setting off: the view stays.
        rig.opts.playhead = 50 * u64::from(PPQ);
        rig.opts.continuous = false;
        for _ in 0..60 {
            rig.frame(vec![]);
        }
        assert!((rig.roll.scroll.unwrap().x - left(10.0)).abs() < 1e-2);
        // A clip no longer than the view does not follow.
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 8.0;
        rig.opts.continuous = true;
        rig.roll.scroll = Some(vec2(0.0, SCROLL_Y));
        rig.frame(vec![]);
        assert_eq!(rig.roll.scroll.unwrap().x, 0.0);
    }

    #[test]
    fn while_recording_the_view_follows_the_unfolded_playhead_or_turns_a_page_with_the_setting_off()
    {
        // Honeydew's own (issue #26): a take's one-bar clip, the playhead 42 beats on from its start, past its loop.
        let mut rig = Rig::new(vec![]);
        (rig.opts.playing, rig.opts.recording, rig.opts.continuous) = (true, true, true);
        rig.opts.playhead = 42 * u64::from(PPQ);
        rig.frame(vec![]);
        // 1120 points show 17.5 beats; beat 42 a quarter in.
        let left = |beat: f32| (beat - 1120.0 / 64.0 / 4.0) * 64.0;
        assert!((rig.roll.scroll.unwrap().x - left(42.0)).abs() < 1e-2);
        // The setting off: the view stays while the playhead is in it, then turns a page.
        rig.opts.continuous = false;
        rig.opts.playhead = 55 * u64::from(PPQ);
        rig.frame(vec![]);
        assert!((rig.roll.scroll.unwrap().x - left(42.0)).abs() < 1e-2);
        rig.opts.playhead = 56 * u64::from(PPQ);
        rig.frame(vec![]);
        assert_eq!(rig.roll.scroll.unwrap().x, 56.0 * 64.0);
    }

    #[test]
    fn a_ruler_press_dragged_sideways_before_its_long_press_scrolls_and_starts_no_range() {
        let mut rig = Rig::new(vec![]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        let from = ruler(1024.0);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(from - vec2(40.0, 0.0))]);
        rig.frame(vec![In::PointerMoved(from - vec2(41.0, 0.0))]);
        rig.frame_after(0.5, vec![]);
        rig.button(from - vec2(41.0, 0.0), PointerButton::Primary, false);
        assert_eq!(rig.song.play_loop, None);
        assert_eq!(rig.roll.scroll.unwrap().x, 41.0);
        // Two fingers there zoom both axes: the roll's touch zones both start at the ruler's top.
        let (beat_u, row_u) = (rig.roll.beat_u, rig.roll.row_u);
        let touch = |id, phase, pos| In::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(id),
            phase,
            pos,
            force: None,
        };
        let (a, b) = (ruler(512.0), ruler(512.0) + vec2(200.0, 100.0));
        rig.frame(vec![In::PointerMoved(a)]);
        rig.frame(vec![
            touch(1, TouchPhase::Start, a),
            In::PointerButton {
                pos: a,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ]);
        rig.frame(vec![touch(2, TouchPhase::Start, b)]);
        rig.frame(vec![touch(2, TouchPhase::Move, b + vec2(100.0, 50.0))]);
        assert!((rig.roll.beat_u - beat_u * 1.5).abs() < 1e-4);
        assert!((rig.roll.row_u - row_u * 1.5).abs() < 1e-4);
    }

    #[test]
    fn a_long_press_selects_a_note_and_the_floating_delete_removes_it() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        assert_eq!(rig.text("Delete"), None);
        rig.hold(at(140.0, 70));
        rig.release(at(140.0, 70));
        assert_eq!(rig.roll.selected, [true, false]);
        assert!(rig.all.is_empty());
        for label in ["Copy", "Delete", "Snap", "More..."] {
            assert!(rig.text(label).is_some(), "no {label}");
        }
        let delete = rig.text("Delete").unwrap();
        rig.tap(delete);
        assert_eq!(spans(rig.events()), [(256, 64.0, 60)]);
        assert_eq!(rig.roll.selected, [false]);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn a_right_press_deletes_its_note_and_a_right_drag_every_note_its_path_crosses_in_one_undo_step()
     {
        // The right-button delete (`gesture::Erase`).
        let mut rig = Rig::new(vec![
            n(128, 64.0, 70),
            n(256, 64.0, 66),
            n(384, 64.0, 60),
            n(512, 64.0, 66),
        ]);
        // On empty grid a right click deletes nothing (and, with no note clipboard, shows no `Paste`).
        right_click(&mut rig, at(700.0, 66));
        assert_eq!(rig.events().len(), 4);
        assert!(rig.all.is_empty());
        // The press deletes the note under it at once.
        let p = at(140.0, 70);
        rig.frame(vec![In::PointerMoved(p)]);
        rig.button(p, PointerButton::Secondary, true);
        assert_eq!(rig.events().len(), 3);
        // Down to key 66 short of its notes, then along it past both in one frame: the path between the frames
        // deletes the one the pointer never stopped on too; key 60's note stays.
        rig.frame(vec![In::PointerMoved(at(140.0, 66))]);
        rig.frame(vec![In::PointerMoved(at(600.0, 66))]);
        rig.button(at(600.0, 66), PointerButton::Secondary, false);
        assert_eq!(spans(rig.events()), [(384, 64.0, 60)]);
        assert_eq!(rig.roll.selected, [false]);
        // Ongoing edits, then one final: one undo step.
        assert_eq!(rig.all, [EDITED, EDITED, DONE]);
        assert_eq!(rig.text("Paste"), None);
    }

    #[test]
    fn dragging_a_selected_note_moves_the_whole_selection() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60), n(384, 32.0, 50)]);
        rig.roll.selected = vec![true, true, false];
        let from = at(140.0, 70);
        rig.drag(from, from + vec2(64.0, 16.0));
        assert_eq!(
            spans(rig.events()),
            [(256, 64.0, 69), (384, 64.0, 59), (384, 32.0, 50)]
        );
        assert_eq!(rig.roll.selected, [true, true, false]);
    }

    #[test]
    fn a_long_press_on_empty_space_then_a_drag_boxes_notes() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 68), n(384, 32.0, 60)]);
        let from = at(100.0, 71);
        rig.hold(from);
        let to = at(300.0, 67);
        rig.frame(vec![In::PointerMoved(to)]);
        rig.release(to);
        assert_eq!(rig.roll.selected, [true, true, false]);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn floating_copy_places_the_copies_after_the_selection() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.select(vec![true]);
        let copy = rig.text("Copy").unwrap();
        rig.tap(copy);
        assert_eq!(
            spans(&super::super::note_clipboard(&rig.ctx)),
            [(128, 64.0, 70)]
        );
        assert_eq!(spans(rig.events()), [(128, 64.0, 70), (192, 64.0, 70)]);
        assert_eq!(rig.roll.selected, [false, true]);
        assert_eq!(rig.all, [DONE]);
    }

    /// Escape: closes the open popup.
    fn escape(rig: &mut Rig) {
        rig.frame(vec![In::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        rig.frame(vec![]);
    }

    /// The grid at the rig's size, and a right click.
    const GRID: Rect = Rect::from_min_max(pos2(50.0, TOP), pos2(1170.0, 349.0));

    fn right_click(rig: &mut Rig, p: Pos2) {
        rig.frame(vec![In::PointerMoved(p)]);
        rig.button(p, PointerButton::Secondary, true);
        rig.button(p, PointerButton::Secondary, false);
    }

    /// The first note's rect in the roll's current view, and where the shared rule puts the floating row over it
    /// with the finger at `finger` (at U = 32 a 4-button row is 204.8 × 48, m 6.4).
    fn shared_row(rig: &Rig, finger: Pos2) -> (Rect, Option<Rect>) {
        let g = rig.roll.geom(GRID.min, 32.0);
        let e = &rig.events()[0];
        let rect = g.note_rect(e).unwrap();
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let handles = [crate::ui::clips::end_handle(rect, &m)];
        let sel = Selection {
            rect,
            one_row: true,
            handles: &handles,
        };
        (
            rect,
            place_row(&sel, finger, GRID, vec2(204.8, 48.0), 6.4, 1.0),
        )
    }

    #[test]
    fn the_floating_row_hides_on_a_scroll_or_zoom_and_a_tap_on_the_selection_brings_it_back() {
        // The rule the playlist and the drum editor share.
        let mut rig = Rig::new(vec![n(512, 128.0, 66)]);
        let middle = |rig: &Rig| shared_row(rig, Pos2::ZERO).0.center();
        let p = middle(&rig);
        rig.tap(p);
        assert_eq!(rig.roll.selected, [true]);
        assert!(rig.text("Copy").is_some());
        assert_eq!(rig.roll.row.rect(), shared_row(&rig, p).1);
        // The wheel scrolls: hidden, the note still in view.
        rig.frame(vec![
            In::PointerMoved(pos2(600.0, 200.0)),
            In::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, -16.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        assert_ne!(rig.roll.scroll, Some(vec2(0.0, SCROLL_Y)));
        assert!(GRID.contains(middle(&rig)));
        assert_eq!(rig.text("Copy"), None);
        // A long press on the selection: back, at the new place, the note still selected (a tap would deselect it).
        let p = middle(&rig);
        rig.hold(p);
        rig.release(p);
        assert_eq!(rig.roll.selected, [true]);
        assert!(rig.text("Copy").is_some());
        assert_eq!(rig.roll.row.rect(), shared_row(&rig, p).1);
        // A pinch zooms: hidden again.
        rig.frame(vec![In::PointerMoved(pos2(600.0, 200.0)), In::Zoom(1.5)]);
        rig.frame(vec![]);
        assert_eq!(rig.text("Copy"), None);
        assert_eq!(rig.roll.selected, [true]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    #[test]
    fn the_rolls_floating_row_is_the_shared_placement() {
        // Each: a note (tick, key) tapped at its middle, and where its row went.
        for ((tick, key), side) in [
            // Room on both sides, one key high: above.
            ((512, 66), "above"),
            // The top row: no room above, so below.
            ((512, 72), "below"),
            // At the left edge: centred on the finger, slid right into the grid; above.
            ((0, 66), "above"),
        ] {
            let mut rig = Rig::new(vec![n(tick, 64.0, key)]);
            let (note, _) = shared_row(&rig, Pos2::ZERO);
            rig.tap(note.center());
            let (_, want) = shared_row(&rig, note.center());
            let row = rig.roll.row.rect();
            assert_eq!(row, want, "key {key}");
            let row = row.unwrap();
            assert!(GRID.contains_rect(row), "key {key}");
            match side {
                "above" => assert_eq!(row.bottom(), note.top() - 6.4, "key {key}"),
                _ => assert_eq!(row.top(), note.bottom() + 6.4, "key {key}"),
            }
            // The labels sit in the row's 51.2-point slots, left to right.
            for (i, label) in ["Copy", "Delete", "Snap", "More..."]
                .into_iter()
                .enumerate()
            {
                let at = rig.text(label).unwrap();
                let slot = pos2(row.left() + 25.6 + 51.2 * i as f32, row.center().y);
                assert!(
                    (at - slot).length() < 1.0,
                    "{label} at {at:?}, not {slot:?}"
                );
            }
        }
    }

    #[test]
    fn more_opens_the_note_menu_and_set_length_applies() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.select(vec![true]);
        let more = rig.text("More...").unwrap();
        rig.tap(more);
        rig.frame(vec![]);
        for label in [
            "Legato",
            "Guess Scale...",
            "Toggle Slide",
            "Transpose by...",
            "Select All",
        ] {
            assert!(rig.text(label).is_some(), "no {label}");
        }
        let item = rig.text("Set length 1/4").unwrap();
        rig.tap(item);
        assert_eq!(spans(rig.events()), [(128, 128.0, 70)]);
        assert_eq!(rig.all, [DONE]);
        rig.frame(vec![]);
        assert_eq!(rig.text("Set length 1/4"), None, "the menu stayed open");
    }

    #[test]
    fn the_snap_button_toggles_and_its_long_press_picks_a_mode() {
        let mut rig = Rig::new(vec![]);
        rig.knob_away();
        let button = pos2(24.0, 24.0);
        rig.tap(button);
        assert!(!rig.roll.snap.on);
        rig.tap(button);
        assert!(rig.roll.snap.on);
        rig.hold(button);
        rig.release(button);
        rig.frame(vec![]);
        assert!(rig.roll.snap.on, "a long press does not toggle");
        let beat = rig.text("Beat").expect("the snap menu is open");
        rig.tap(beat);
        assert_eq!(rig.roll.snap.mode, SnapMode::Beat);
        rig.frame(vec![]);
        assert_eq!(rig.text("Beat"), None);
    }

    #[test]
    fn black_0_15_covers_the_grid_past_the_loop_end_only_while_that_end_is_left_of_the_grid_edge() {
        // A one-bar clip: its loop ends 4 beats × 64 right of the grid's left edge (tick 0).
        let mut rig = Rig::new(vec![n(0, 64.0, 60)]);
        let grid = rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("grid"))
            .unwrap()
            .rect;
        let past = |rig: &Rig| -> Vec<Rect> {
            (rig.painted.iter())
                .filter(|(_, fill)| *fill == egui::Color32::from_black_alpha(38))
                .map(|(r, _)| *r)
                .collect()
        };
        assert_eq!(past(&rig), [grid.with_min_x(grid.left() + 256.0)]);
        assert_eq!(grid.left() + 256.0, at(512.0, 60).x);
        // Eight bars: the end lies past the grid's right edge, and nothing darkens.
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        rig.frame(vec![]);
        assert_eq!(past(&rig), []);
    }

    #[test]
    fn a_0_35_s_mouse_hold_on_the_snap_button_opens_its_menu_without_toggling() {
        // The editors' 300 ms long press, not egui's 0.8 s.
        let mut rig = Rig::new(vec![]);
        rig.knob_away();
        let button = pos2(24.0, 24.0);
        rig.frame(vec![In::PointerMoved(button)]);
        rig.button(button, PointerButton::Primary, true);
        rig.frame_after(0.35, vec![]);
        rig.release(button);
        rig.frame(vec![]);
        assert!(rig.roll.snap.on, "a long press does not toggle");
        assert!(rig.text("Beat").is_some(), "the snap menu is open");
    }

    #[test]
    fn the_wheel_scrolls_and_zoom_keeps_the_point_under_the_pointer() {
        let mut rig = Rig::new(vec![]);
        // Eight bars, so that zooming in leaves room to scroll.
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        let p = at(256.0, 66);
        rig.frame(vec![In::PointerMoved(p)]);
        rig.frame(vec![In::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -48.0),
            modifiers: Modifiers::NONE,
        }]);
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        let scroll = rig.roll.scroll.unwrap();
        assert!(
            (scroll - vec2(0.0, SCROLL_Y + 48.0)).length() < 0.01,
            "{scroll:?}"
        );
        let p = pos2(600.0, 200.0);
        let before = rig.roll.geom(pos2(50.0, TOP), 32.0);
        // A trackpad pinch over the grid zooms the time only.
        rig.frame(vec![In::PointerMoved(p), In::Zoom(2.0)]);
        assert_eq!((rig.roll.beat_u, rig.roll.row_u), (4.0, 0.5));
        let after = rig.roll.geom(pos2(50.0, TOP), 32.0);
        assert!((after.tick(p.x) - before.tick(p.x)).abs() < 1e-3);
        assert!(after.key(p.y) == before.key(p.y));
    }

    #[test]
    fn the_rolls_wheel_zooms_the_rows_on_the_keys_the_time_on_the_ruler_and_scrolls_on_the_grid() {
        // The editor's wheel zones, one raw wheel turn each.
        let mut rig = Rig::new(vec![]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
        let wheel = |rig: &mut Rig, at: Pos2, delta: Vec2| {
            rig.frame(vec![
                In::PointerMoved(at),
                In::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta,
                    modifiers: Modifiers::NONE,
                },
            ]);
        };
        let g = |rig: &Rig| rig.roll.geom(pos2(50.0, TOP), 32.0);
        let row_at = |g: Geom, y: f32| (y - g.origin.y) / g.row_h;
        let tick_at = |g: Geom, x: f32| g.tick(x);
        // The key column: the rows, the row under the pointer kept.
        let p = pos2(20.0, 200.0);
        let (before, scroll_x) = (row_at(g(&rig), p.y), rig.roll.scroll.unwrap().x);
        wheel(&mut rig, p, vec2(0.0, 200.0));
        assert_eq!((rig.roll.beat_u, rig.roll.row_u), (2.0, 0.625));
        assert!((row_at(g(&rig), p.y) - before).abs() < 1e-4);
        assert_eq!(rig.roll.scroll.unwrap().x, scroll_x);
        // The ruler: the time, the tick under the pointer kept.
        let p = pos2(600.0, 24.0);
        let (before, scroll_y) = (tick_at(g(&rig), p.x), rig.roll.scroll.unwrap().y);
        wheel(&mut rig, p, vec2(0.0, 200.0));
        assert_eq!((rig.roll.beat_u, rig.roll.row_u), (2.5, 0.625));
        assert!((tick_at(g(&rig), p.x) - before).abs() < 1e-3);
        assert_eq!(rig.roll.scroll.unwrap().y, scroll_y);
        // The grid: it scrolls, the zooms as they were.
        let before = rig.roll.scroll.unwrap();
        wheel(&mut rig, pos2(600.0, 200.0), vec2(-40.0, -30.0));
        assert_eq!(rig.roll.scroll.unwrap(), before + vec2(40.0, 30.0));
        assert_eq!((rig.roll.beat_u, rig.roll.row_u), (2.5, 0.625));
    }

    #[test]
    fn the_key_column_plays_its_keys_and_never_scrolls() {
        let mut rig = Rig::new(vec![]);
        let scroll = rig.roll.scroll;
        let c5 = at(0.0, 72) - vec2(at(0.0, 72).x - 20.0, 0.0);
        rig.frame(vec![In::PointerMoved(c5)]);
        rig.button(c5, PointerButton::Primary, true);
        assert_eq!(rig.sounds, [(72, Some(crate::app::KEY_VELOCITY))]);
        // Down a row: the next key; the column does not scroll.
        rig.frame(vec![In::PointerMoved(c5 + vec2(0.0, 16.0))]);
        assert_eq!(
            rig.sounds[1..],
            [(72, None), (71, Some(crate::app::KEY_VELOCITY))]
        );
        rig.release(c5 + vec2(0.0, 16.0));
        assert_eq!(rig.sounds.last(), Some(&(71, None)));
        assert_eq!(rig.roll.scroll, scroll);
    }

    // A vertical key column. Key 72 (`C6`) is the top row, y TOP … TOP + 16.

    /// `rig` showed a `fill` rect at `want` (to a thousandth of a point) last frame.
    fn shows(rig: &Rig, want: Rect, fill: egui::Color32) -> bool {
        (rig.visible.iter()).any(|&(r, f)| {
            f == fill && (r.min - want.min).length() < 1e-3 && (r.max - want.max).length() < 1e-3
        })
    }

    /// The column's label texts, with their fill and font size.
    fn key_labels(rig: &Rig) -> Vec<(String, egui::Color32, Rect, f32)> {
        (rig.shapes.iter())
            .filter_map(|s| match s {
                egui::Shape::Text(t) if t.pos.x < 48.0 => Some((
                    t.galley.text().to_owned(),
                    t.fallback_color,
                    Rect::from_min_size(t.pos, t.galley.size()),
                    t.galley.job.sections[0].format.font_id.size,
                )),
                _ => None,
            })
            .collect()
    }

    /// Row height `row_u` U, `C6` (key 72) the top row.
    fn rows_at(rig: &mut Rig, row_u: f32) {
        rig.roll.row_u = row_u;
        rig.roll.scroll = Some(vec2(0.0, 55.0 * row_u * 32.0));
        rig.frame(vec![]);
    }

    #[test]
    fn a_black_keys_row_spans_the_whole_key_column_and_plays_its_key_there() {
        let mut rig = Rig::new(vec![]);
        let black = theme_by_id(0).colors.black_key.to_color32();
        // A#5 (70): the third row, the column's full 1.5 U.
        let row = Rect::from_x_y_ranges(0.0..=48.0, TOP + 32.0..=TOP + 48.0);
        assert!(shows(&rig, row, black), "{:?}", rig.visible);
        // Its right end plays it.
        let p = pos2(46.0, at(0.0, 70).y);
        rig.frame(vec![In::PointerMoved(p)]);
        rig.button(p, PointerButton::Primary, true);
        assert_eq!(rig.sounds, [(70, Some(crate::app::KEY_VELOCITY))]);
        rig.release(p);
    }

    #[test]
    fn two_white_keys_meet_on_a_1_px_background_line_at_a_third_and_no_gap() {
        let rig = Rig::new(vec![]);
        let c = theme_by_id(0).colors;
        // Under C6, over B5.
        let line = Rect::from_x_y_ranges(0.0..=48.0, TOP + 15.0..=TOP + 16.0);
        let faint = c.background.to_color32().gamma_multiply(0.3);
        assert!(shows(&rig, line, faint), "{:?}", rig.visible);
        // The line lies on C6's full row.
        let c6 = Rect::from_x_y_ranges(0.0..=48.0, TOP..=TOP + 16.0);
        assert!(shows(&rig, c6, c.white_key.to_color32()));
    }

    #[test]
    fn a_c_row_has_a_channel_colour_bar_down_its_left_edge() {
        let rig = Rig::new(vec![]);
        let theme = theme_by_id(0);
        let ink = crate::theme::channel_color(rig.song.channels[1].hue, &theme).to_color32();
        // ceil(0.1 U) = 4 px wide, the row's full height: C6 (the top row) and C5, 12 rows down.
        for top in [TOP, TOP + 12.0 * 16.0] {
            let bar = Rect::from_x_y_ranges(0.0..=4.0, top..=top + 16.0);
            assert!(shows(&rig, bar, ink), "{top}: {:?}", rig.visible);
        }
    }

    #[test]
    fn a_c_label_starts_just_right_of_the_bar_centred_on_its_row_in_black_key_disabled() {
        let rig = Rig::new(vec![]);
        let grey = theme_by_id(0).colors.black_key_disabled.to_color32();
        let labels = key_labels(&rig);
        let (_, color, rect, size) = (labels.iter())
            .find(|l| l.0 == "C6")
            .unwrap_or_else(|| panic!("{labels:?}"));
        assert_eq!(*color, grey);
        // 0.08 U after the 4 px bar; centred on the row's middle.
        assert!((rect.left() - (4.0 + 0.08 * 32.0)).abs() < 1e-3, "{rect:?}");
        assert!((rect.center().y - (TOP + 8.0)).abs() <= 0.5, "{rect:?}");
        // 0.95 × the 16 px row = 7.6 pt: floor(32 · 7.6 / 16) = 15 px.
        assert_eq!(*size, 15.0);
    }

    #[test]
    fn the_label_grows_with_the_rows_up_to_12_pt_and_short_rows_keep_the_bar_without_it() {
        let mut rig = Rig::new(vec![]);
        let theme = theme_by_id(0);
        let ink = crate::theme::channel_color(rig.song.channels[1].hue, &theme).to_color32();
        let size = |rig: &Rig| key_labels(rig).iter().find(|l| l.0 == "C6").map(|l| l.3);
        // 2 U rows: 0.95 × 64 px is over 12 pt (24 px).
        rows_at(&mut rig, 2.0);
        assert_eq!(size(&rig), Some(24.0));
        // 0.375 U rows: 0.95 × 12 px is under 6 pt (12 px): no label, the bar stays.
        rows_at(&mut rig, 0.375);
        assert!(key_labels(&rig).is_empty(), "{:?}", key_labels(&rig));
        let bar = Rect::from_x_y_ranges(0.0..=4.0, TOP..=TOP + 12.0);
        assert!(shows(&rig, bar, ink), "{:?}", rig.visible);
        // 0.4375 U rows (14 px): 0.95 × 14 = 13.3 px, 6.65 pt.
        rows_at(&mut rig, 0.4375);
        assert_eq!(size(&rig), Some(13.0));
    }

    #[test]
    fn ghost_notes_follow_the_setting() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        song.add_channel(ChannelKind::Instrument);
        song.channels[1].lanes[0].add_clip(0, 4.0);
        song.channels[2].lanes[0].add_clip(0, 4.0).events = vec![n(0, 32.0, 60)];
        let roll = PianoRoll::for_channel(&song, 1);
        let on = RollOptions {
            ghost_notes: true,
            ..Default::default()
        };
        let all = || f64::NEG_INFINITY..f64::INFINITY;
        assert_eq!(roll.ghosts(&song, &on, all(), 0.0).len(), 1);
        assert!(
            roll.ghosts(&song, &RollOptions::default(), all(), 0.0)
                .is_empty()
        );
    }

    #[test]
    fn a_touch_long_press_on_a_note_selects_it_and_dragging_on_moves_the_selection() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        let p = at(140.0, 70);
        rig.touch_hold(p);
        assert_eq!(rig.roll.selected, [true, false]);
        assert!(rig.all.is_empty());
        // Still down: dragging on moves the selection by a beat.
        rig.touch(1.0 / 60.0, TouchPhase::Move, p + vec2(32.0, 0.0));
        rig.touch(1.0 / 60.0, TouchPhase::Move, p + vec2(64.0, 0.0));
        rig.touch(1.0 / 60.0, TouchPhase::End, p + vec2(64.0, 0.0));
        assert_eq!(spans(rig.events()), [(256, 64.0, 70), (256, 64.0, 60)]);
        assert_eq!(rig.roll.selected, [true, false]);
        assert_eq!(rig.last, [DONE]);
    }

    #[test]
    fn a_pinch_that_starts_on_a_note_zooms_and_leaves_the_note_where_it_was() {
        let p = at(140.0, 70);
        let q = p + vec2(200.0, 0.0);
        // The fingers spread and the second one lifts first, then the first moves on.
        let pinch = |rig: &mut Rig, from: Pos2| {
            rig.second(TouchPhase::Start, q);
            rig.touch(1.0 / 60.0, TouchPhase::Move, from - vec2(64.0, 0.0));
            rig.second(TouchPhase::Move, q + vec2(64.0, 0.0));
            rig.second(TouchPhase::End, q + vec2(64.0, 0.0));
            rig.touch(1.0 / 60.0, TouchPhase::Move, from - vec2(96.0, 0.0));
            rig.touch(1.0 / 60.0, TouchPhase::End, from - vec2(96.0, 0.0));
        };
        // The second finger lands before the first one moves.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.touch(1.0 / 60.0, TouchPhase::Start, p);
        pinch(&mut rig, p);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert!(rig.roll.beat_u > BEAT_U, "{}", rig.roll.beat_u);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
        // The first finger is already dragging the note a beat right: the second puts it back and ends the gesture.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.touch(1.0 / 60.0, TouchPhase::Start, p);
        rig.touch(1.0 / 60.0, TouchPhase::Move, p + vec2(64.0, 0.0));
        assert_eq!(spans(rig.events()), [(256, 64.0, 70)]);
        pinch(&mut rig, p + vec2(64.0, 0.0));
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert_eq!(rig.all, [EDITED, DONE]);
    }

    #[test]
    fn a_focus_loss_ends_a_drag_and_a_stale_press_drags_nothing() {
        // egui keeps the button down over a focus loss: the drag ends there, and hovering afterwards moves nothing.
        let from = at(140.0, 70);
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(from + vec2(64.0, 0.0))]);
        assert_eq!(spans(rig.events()), [(256, 64.0, 70)]);
        rig.frame(vec![In::WindowFocused(false)]);
        rig.frame(vec![
            In::WindowFocused(true),
            In::PointerMoved(from + vec2(192.0, 32.0)),
        ]);
        rig.frame(vec![In::PointerMoved(from + vec2(256.0, 0.0))]);
        assert_eq!(spans(rig.events()), [(256, 64.0, 70)]);
        assert_eq!(rig.all, [EDITED, DONE]);
        // A press gone stale before it moved: the hover that egui takes for its drag moves nothing either.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        rig.frame(vec![In::WindowFocused(false)]);
        rig.frame(vec![In::PointerMoved(from + vec2(64.0, 0.0))]);
        rig.frame(vec![In::PointerMoved(from + vec2(128.0, 0.0))]);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    #[test]
    fn a_quick_pinch_whose_first_finger_stays_put_is_no_tap() {
        // The first finger lifts where it landed, within the click time: egui reports a click there.
        let quick_pinch = |rig: &mut Rig, at: Pos2| {
            let q = at + vec2(200.0, 0.0);
            rig.touch(1.0 / 60.0, TouchPhase::Start, at);
            rig.second(TouchPhase::Start, q);
            rig.second(TouchPhase::Move, q + vec2(64.0, 0.0));
            rig.second(TouchPhase::End, q + vec2(64.0, 0.0));
            rig.touch(1.0 / 60.0, TouchPhase::End, at);
        };
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        // On a note (a tap would select it), then on an empty cell (a tap would add one).
        quick_pinch(&mut rig, at(140.0, 70));
        assert!(rig.roll.beat_u > BEAT_U, "no pinch");
        let empty = at(140.0, 66);
        quick_pinch(&mut rig, empty);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
        // A tap on the empty cell still adds a note.
        rig.tap(empty);
        assert_eq!(rig.events().len(), 2);
        assert_eq!(rig.all, [QUIET]);
    }

    #[test]
    fn a_move_that_arrives_with_the_lift_still_applies() {
        // The finger moves and lifts within one frame: the pointer is gone by the end of it, but the move counts.
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 60)]);
        let p = at(140.0, 70);
        rig.touch_hold(p);
        let to = p + vec2(64.0, 0.0);
        let finger = |phase| In::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos: to,
            force: None,
        };
        rig.frame(vec![
            finger(TouchPhase::Move),
            In::PointerMoved(to),
            finger(TouchPhase::End),
            In::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            In::PointerGone,
        ]);
        assert_eq!(spans(rig.events()), [(256, 64.0, 70), (256, 64.0, 60)]);
        assert_eq!(rig.last, [DONE]);
        // A box the same way.
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 68), n(384, 32.0, 60)]);
        let from = at(100.0, 71);
        rig.touch_hold(from);
        let to = at(300.0, 67);
        rig.frame(vec![
            In::PointerMoved(to),
            In::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            In::PointerGone,
        ]);
        assert_eq!(rig.roll.selected, [true, true, false]);
    }

    /// A filled rect of about `size` centred on `at` was drawn last frame.
    fn square_at(rig: &Rig, at: Pos2, size: f32) -> bool {
        (rig.rects.iter())
            .any(|r| (r.center() - at).length() < 0.5 && (r.width() - size).abs() < 0.5)
    }

    #[test]
    fn a_held_touch_on_empty_grid_grows_the_marquee_and_its_drag_selects_live() {
        // The marquee, with the mouse held.
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 68), n(384, 32.0, 60)]);
        rig.tap(at(140.0, 70));
        assert_eq!(rig.roll.selected, [true, false, false]);
        let from = at(100.0, 71);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        // 0.2 s in: a square of 3 U·(4.4333·0.2 − 0.33) on the touch-down point.
        rig.frame_after(0.2, vec![]);
        assert!(
            square_at(&rig, from, 96.0 * (0.2 * 4.4333 - 0.33)),
            "{:?}",
            rig.rects
        );
        // Past 300 ms: no marquee until dragged, and nothing selected.
        rig.frame_after(0.15, vec![]);
        assert!(!square_at(&rig, from, 96.0));
        assert_eq!(rig.roll.selected, [false; 3]);
        // A drag: the rectangle to the finger, whose notes are the selection before the lift.
        let to = at(300.0, 67);
        rig.frame(vec![In::PointerMoved(to)]);
        assert_eq!(rig.roll.selected, [true, true, false]);
        assert!(rig.rects.contains(&Rect::from_two_pos(from, to)));
        rig.release(to);
        assert_eq!(rig.roll.selected, [true, true, false]);
        assert!(rig.text("Copy").is_some());
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_mouse_hold_of_0_35_s_is_a_long_press_and_of_0_25_s_a_tap() {
        // The editors' own 300 ms classifier, not egui's 0.8 s drag start.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let p = at(140.0, 70);
        rig.tap(p);
        assert_eq!(rig.roll.selected, [true]);
        // Held 0.35 s on the selected note: a long press, which keeps it.
        rig.frame(vec![In::PointerMoved(p)]);
        rig.button(p, PointerButton::Primary, true);
        rig.frame_after(0.35, vec![]);
        rig.release(p);
        assert_eq!(rig.roll.selected, [true]);
        // Held 0.25 s: a tap, which deselects it.
        rig.button(p, PointerButton::Primary, true);
        rig.frame_after(0.25, vec![]);
        rig.release(p);
        assert_eq!(rig.roll.selected, [false]);
        // On empty grid, 0.35 s adds nothing (a long press: the marquee) and 0.25 s adds a note.
        let empty = at(400.0, 66);
        rig.frame(vec![In::PointerMoved(empty)]);
        rig.button(empty, PointerButton::Primary, true);
        rig.frame_after(0.35, vec![]);
        rig.release(empty);
        assert_eq!(rig.events().len(), 1);
        rig.button(empty, PointerButton::Primary, true);
        rig.frame_after(0.25, vec![]);
        rig.release(empty);
        assert_eq!(rig.events().len(), 2);
    }

    #[test]
    fn the_shared_long_press_time_and_calm_zone_decide_a_touch_on_empty_grid() {
        // `gesture::LONG_PRESS_S` and `CALM_U` (0.3 s; 0.5 U, 16 points at U = 32): released at 0.28 s, or moved
        // 0.45 U, a press on empty grid is a tap (a note); at 0.32 s it is a long press, 0.55 U out a scroll: nothing.
        let notes_after = |secs: f64, by: f32| {
            let mut rig = Rig::new(vec![]);
            let p = at(400.0, 66);
            rig.frame(vec![In::PointerMoved(p)]);
            rig.button(p, PointerButton::Primary, true);
            let (mut to, mut held) = (p, 0.0);
            if by != 0.0 {
                to = p + vec2(by, 0.0);
                rig.frame_after(0.1, vec![In::PointerMoved(to)]);
                held = 0.1;
            }
            let up = In::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            };
            rig.frame_after(secs - held, vec![up]);
            rig.events().len()
        };
        assert_eq!(notes_after(0.28, 0.0), 1);
        assert_eq!(notes_after(0.32, 0.0), 0);
        assert_eq!(notes_after(0.2, 0.45 * 32.0), 1);
        assert_eq!(notes_after(0.2, 0.55 * 32.0), 0);
    }

    #[test]
    fn a_long_press_marquee_is_a_box_only_past_the_calm_zone() {
        // After the long press, the marquee is the 3 U square on the finger (selecting what lies under it) until the
        // finger leaves the 0.5 U calm zone (16 points at U = 32); then it is the box from the touch-down. A note of
        // 10 ticks (5 points) lies between the two, under neither finger.
        let selected_after = |by: f32| {
            let mut rig = Rig::new(vec![n(410, 10.0, 66)]);
            let p = at(400.0, 66);
            rig.frame(vec![In::PointerMoved(p)]);
            rig.button(p, PointerButton::Primary, true);
            rig.frame_after(0.35, vec![]);
            rig.frame(vec![In::PointerMoved(p + vec2(by, 0.0))]);
            rig.roll.selected.clone()
        };
        assert_eq!(selected_after(0.45 * 32.0), [false]);
        assert_eq!(selected_after(0.55 * 32.0), [true]);
    }

    #[test]
    fn a_touch_long_press_on_empty_space_then_a_move_boxes_notes() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(256, 64.0, 68), n(384, 32.0, 60)]);
        let from = at(100.0, 71);
        rig.touch_hold(from);
        let to = at(300.0, 67);
        rig.touch(1.0 / 60.0, TouchPhase::Move, from + (to - from) / 2.0);
        rig.touch(1.0 / 60.0, TouchPhase::Move, to);
        rig.touch(1.0 / 60.0, TouchPhase::End, to);
        assert_eq!(rig.roll.selected, [true, true, false]);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_touch_long_press_on_the_snap_button_opens_the_menu() {
        let mut rig = Rig::new(vec![]);
        rig.knob_away();
        let button = pos2(24.0, 24.0);
        rig.touch_hold(button);
        rig.touch(1.0 / 60.0, TouchPhase::End, button);
        rig.frame(vec![]);
        assert!(rig.roll.snap.on, "a long press does not toggle");
        assert!(rig.text("Beat").is_some(), "the snap menu is not open");
    }

    #[test]
    fn a_long_press_on_an_off_grid_note_only_selects_it() {
        let mut rig = Rig::new(vec![n(40, 64.0, 70)]);
        let p = at(60.0, 70);
        rig.hold(p);
        rig.release(p);
        assert_eq!(spans(rig.events()), [(40, 64.0, 70)]);
        assert_eq!(rig.roll.selected, [true]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
        // The same by touch, after clearing the selection.
        rig.tap(at(600.0, 66));
        rig.touch_hold(p);
        rig.touch(1.0 / 60.0, TouchPhase::Move, p + vec2(2.0, 1.0));
        rig.touch(1.0 / 60.0, TouchPhase::End, p + vec2(2.0, 1.0));
        assert_eq!(spans(rig.events()), [(40, 64.0, 70)]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    #[test]
    fn a_long_press_on_a_handle_does_not_resize() {
        let mut rig = Rig::new(vec![n(40, 64.0, 70)]);
        rig.roll.selected = vec![true];
        rig.frame(vec![]);
        // The note ends at tick 104 (x 100); the handle's centre is 0.3 U further.
        let handle = at(104.0, 70) + vec2(0.3 * 32.0, 0.0);
        rig.hold(handle);
        rig.release(handle);
        assert_eq!(spans(rig.events()), [(40, 64.0, 70)]);
        assert!(rig.all.is_empty(), "{:?}", rig.all);
    }

    /// Channel 1 with a 4-beat clip whose visible length is set to its loop length.
    fn visible_clip(notes: Vec<Event>) -> Rig {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
        clip.visible_len_beats = 4.0;
        clip.events = notes;
        Rig::with_song(song)
    }

    #[test]
    fn growing_the_loop_keeps_a_visible_length_that_matched_it() {
        // A clip that has been sized keeps its length, so the note beyond it is silent.
        let mut rig = visible_clip(vec![]);
        rig.tap(at(600.0, 70));
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (8.0, 4.0));
        let played: Vec<u64> = clip.events_in_song_range(0, 4096).map(|(t, _)| t).collect();
        assert_eq!(played, Vec::<u64>::new());
    }

    #[test]
    fn growing_the_loop_grows_a_new_clips_length() {
        // A new clip has visible length 0: its length is the loop.
        let mut rig = visible_clip(vec![]);
        rig.song.channels[1].lanes[0].clips[0].visible_len_beats = 0.0;
        rig.tap(at(600.0, 70));
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (8.0, 0.0));
        assert_eq!(clip.visible_ticks(), 8.0 * 128.0);
        let played: Vec<u64> = clip.events_in_song_range(0, 4096).map(|(t, _)| t).collect();
        assert_eq!(played, [576]);
    }

    #[test]
    fn a_visible_length_of_its_own_is_kept() {
        let mut rig = visible_clip(vec![]);
        rig.song.channels[1].lanes[0].clips[0].visible_len_beats = 2.0;
        rig.tap(at(600.0, 70));
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (8.0, 2.0));
    }

    #[test]
    fn a_drag_past_the_end_and_back_restores_loop_and_visible_length() {
        let mut rig = visible_clip(vec![n(64, 64.0, 70)]);
        let from = at(80.0, 70);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(from + vec2(300.0, 0.0))]);
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (8.0, 4.0));
        let back = from + vec2(30.0, 0.0);
        rig.frame(vec![In::PointerMoved(back)]);
        rig.release(back);
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!(spans(&clip.events), [(128, 64.0, 70)]);
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (4.0, 4.0));
    }

    #[test]
    fn deleting_a_note_fits_the_loop() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70), n(600, 32.0, 70)]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 8.0;
        rig.tap(at(610.0, 70));
        rig.tap(rig.text("Delete").unwrap());
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 4.0);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn a_drag_back_to_its_start_neither_fits_nor_reports() {
        let mut rig = Rig::new(vec![n(64, 64.0, 70)]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 16.0;
        let from = at(80.0, 70);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        // Out to 672..736: the loop fits to 8.
        rig.frame(vec![In::PointerMoved(from + vec2(300.0, 0.0))]);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 8.0);
        // Back: the notes and the loop are as they were, and that frame reports nothing.
        rig.frame(vec![In::PointerMoved(from)]);
        assert!(rig.last.is_empty(), "{:?}", rig.last);
        let clip = &rig.song.channels[1].lanes[0].clips[0];
        assert_eq!(spans(&clip.events), [(64, 64.0, 70)]);
        assert_eq!(clip.loop_len_beats, 16.0);
        rig.release(from);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 16.0);
    }

    #[test]
    fn menu_items_that_change_no_note_keep_a_long_loop() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.song.channels[1].lanes[0].clips[0].loop_len_beats = 16.0;
        // More... → Select All, then the floating Snap on a note already on the grid.
        more(&mut rig, "Select All");
        rig.tap(rig.text("Snap").unwrap());
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 16.0);
        // Quantize re-syncs whatever it moved; no undo step, as nothing changed.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
    }

    #[test]
    fn notes_above_key_127_do_not_break_the_roll() {
        let mut rig = Rig::new(vec![n(0, 64.0, 0), n(64, 64.0, 200)]);
        rig.roll.scroll = Some(vec2(0.0, 2048.0));
        rig.frame(vec![]);
        // Select all through the note menu, after selecting the key 0 note.
        rig.select(vec![true, false]);
        rig.tap(rig.text("More...").unwrap());
        rig.frame(vec![]);
        rig.tap(rig.text("Select All").unwrap());
        assert_eq!(rig.roll.selected, [true, false]);
        rig.tap(rig.text("Copy").unwrap());
        assert_eq!(
            spans(rig.events()),
            [(0, 64.0, 0), (64, 64.0, 200), (64, 64.0, 0)]
        );
    }

    #[test]
    fn an_outside_change_to_the_notes_drops_the_selection() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        rig.roll.selected = vec![true];
        rig.frame(vec![]);
        assert_eq!(rig.roll.selected, [true]);
        // Same count, different content (an undo, say).
        rig.song.channels[1].lanes[0].clips[0].events = vec![n(0, 32.0, 60)];
        rig.frame(vec![]);
        assert_eq!(rig.roll.selected, [false]);
        assert_eq!(rig.text("Delete"), None);
    }

    #[test]
    fn a_drum_lane_is_never_edited() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let hit = Event::Drum {
            pos: 0,
            len_beats: 0.25,
            pad: 0,
            velocity: 200,
            pan: 127,
            value: 0x7FFF,
            repeat: 0,
        };
        song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![hit];
        let mut rig = Rig::with_song(song);
        assert_eq!(rig.roll.clip_id, None);
        rig.tap(at(100.0, 70));
        let lane = &rig.song.channels[1].lanes[0];
        assert_eq!(
            (lane.clips.len(), &lane.clips[0].events[..]),
            (1, &[hit][..])
        );
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_stale_clip_id_never_makes_a_second_clip() {
        let mut rig = Rig::new(vec![]);
        rig.roll.clip_id = Some(99);
        rig.tap(at(100.0, 70));
        let lane = &rig.song.channels[1].lanes[0];
        assert_eq!(lane.clips.len(), 1);
        assert!(lane.clips[0].events.is_empty());
        assert!(rig.all.is_empty());
    }

    // ---- note menu: scale, slide, transpose ----

    /// Selects every note, opens `More...` and taps `item`.
    fn more(rig: &mut Rig, item: &str) {
        rig.select(vec![true; rig.events().len()]);
        rig.tap(rig.text("More...").unwrap());
        rig.frame(vec![]);
        rig.tap(rig.text(item).unwrap());
        // A dialog's popup, like any new egui area, is drawn from its second frame.
        rig.frame(vec![]);
        rig.frame(vec![]);
    }

    fn keys(rig: &Rig) -> Vec<u8> {
        spans(rig.events()).iter().map(|s| s.2).collect()
    }

    /// The texts of the last frame that start with `#` (the scale-guess rows).
    fn rows(rig: &Rig) -> Vec<String> {
        let mut rows: Vec<(f32, String)> = (rig.texts.iter())
            .filter(|(t, _)| t.starts_with('#'))
            .map(|(t, r)| (r.top(), t.clone()))
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, t)| t).collect()
    }

    fn key_lock(rig: &Rig) -> (i32, i32) {
        (rig.song.key_root, rig.song.key_scale)
    }

    #[test]
    fn every_note_menu_item_is_enabled() {
        let items = crate::ui::piano_roll::more_items();
        assert_eq!(items.len(), 14);
        assert!(items.iter().all(|i| !i.disabled));
    }

    #[test]
    fn humanize_previews_in_its_panel_and_apply_nudges_the_selected_notes_once() {
        // Eight notes: the chance that a normal draw leaves every start and velocity as it was is negligible.
        let notes: Vec<Event> = (0..8).map(|i| n(64 + i * 64, 32.0, 60 + i as u8)).collect();
        let mut rig = Rig::new(notes.clone());
        more(&mut rig, "Humanize");
        // The panel previews; the song waits for `Apply`.
        assert!(rig.text("Apply").is_some(), "the panel shows");
        assert_eq!(rig.events(), &notes[..]);
        assert!(rig.all.is_empty());
        rig.tap(rig.text("Apply").unwrap());
        rig.frame(vec![]);
        // It re-syncs whatever it changed, and hides the selection.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
        assert_eq!(rig.text("Apply"), None, "the panel closed");
        assert!(rig.roll.selected.iter().all(|s| !s));
        assert_eq!(rig.text("Delete"), None, "the floating buttons are gone");
        let shape = |events: &[Event]| -> Vec<(f64, u8)> {
            spans(events).iter().map(|s| (s.1, s.2)).collect()
        };
        let mut after = shape(rig.events());
        after.sort_by_key(|s| s.1);
        assert_eq!(after, shape(&notes), "lengths and keys stay");
        assert_ne!(rig.events(), &notes[..]);
        assert!(rig.events().is_sorted_by_key(|e| e.pos()), "re-sorted");
    }

    #[test]
    fn snap_to_scale_snaps_the_selection_to_the_project_scale() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 65), n(256, 64.0, 66)]);
        // D major: C goes down to B, F down to E; F# stays.
        (rig.song.key_root, rig.song.key_scale) = (2, super::stored("Major"));
        more(&mut rig, "Snap to Scale");
        assert_eq!(keys(&rig), [59, 64, 66]);
        // It re-syncs whatever it changed.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
        assert_eq!(rig.text("Snap to Scale"), None, "the menu stayed open");
    }

    #[test]
    fn snap_to_scale_on_a_chromatic_song_changes_nothing_but_still_re_syncs() {
        let mut rig = Rig::new(vec![n(0, 64.0, 61)]);
        more(&mut rig, "Snap to Scale");
        assert_eq!(keys(&rig), [61]);
        // The clip is rebuilt, then a re-sync runs, whatever moved.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
    }

    #[test]
    fn toggle_slide_flips_the_selected_notes_slide_flag() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), super::slid(128, 64.0, 62)]);
        more(&mut rig, "Toggle Slide");
        assert_eq!(super::slides(rig.events()), [true, false]);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn transpose_by_drags_an_offset_and_applies_it_on_release() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 70)]);
        more(&mut rig, "Transpose by...");
        assert_eq!(
            rig.text("Transpose by..."),
            None,
            "the menu is in transpose mode"
        );
        let label = rig.text("0").expect("the offset shows 0");
        // Half a U (16 points) a semitone, up is up.
        rig.frame(vec![In::PointerMoved(label)]);
        rig.button(label, PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(label + vec2(0.0, 40.0))]);
        assert!(rig.text("-3").is_some(), "{:?}", rig.texts);
        rig.frame(vec![In::PointerMoved(label - vec2(0.0, 48.0))]);
        assert!(rig.text("+3").is_some(), "{:?}", rig.texts);
        assert_eq!(keys(&rig), [60, 70], "nothing moves before the release");
        assert!(rig.all.is_empty());
        rig.button(label - vec2(0.0, 48.0), PointerButton::Primary, false);
        assert_eq!(keys(&rig), [63, 73]);
        assert_eq!(rig.roll.selected, [true, true]);
        assert_eq!(rig.all, [DONE]);
        rig.frame(vec![]);
        assert_eq!(rig.text("+3"), None, "the transpose popup stayed open");
    }

    #[test]
    fn the_transpose_offset_stops_where_a_note_would_leave_the_keys() {
        let mut rig = Rig::new(vec![n(0, 64.0, 120)]);
        // Scrolled to the top keys: a selection out of view has no floating row, so no `More...`.
        rig.roll.scroll = Some(Vec2::ZERO);
        more(&mut rig, "Transpose by...");
        let label = rig.text("0").unwrap();
        rig.drag(label, label - vec2(0.0, 400.0));
        assert_eq!(keys(&rig), [127]);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn escape_or_a_tap_cancels_the_transpose() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60)]);
        more(&mut rig, "Transpose by...");
        rig.frame(vec![In::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        rig.frame(vec![]);
        assert_eq!(rig.text("0"), None);
        more(&mut rig, "Transpose by...");
        rig.tap(pos2(1000.0, 300.0));
        rig.frame(vec![]);
        assert_eq!(rig.text("0"), None);
        assert_eq!(keys(&rig), [60]);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn with_nothing_selected_transpose_moves_every_note() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 64)]);
        rig.roll.menu = Some(crate::ui::piano_roll::Menu::Transpose {
            at: pos2(600.0, 200.0),
            offset: 0,
            from: None,
        });
        rig.frame(vec![]);
        rig.frame(vec![]);
        let label = rig.text("0").unwrap();
        rig.drag(label, label + vec2(0.0, 32.0));
        assert_eq!(keys(&rig), [58, 62]);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn guess_scale_lists_candidates_and_picking_one_sets_the_project_scale() {
        let mut rig = Rig::new(vec![n(0, 64.0, 62), n(128, 64.0, 66), n(256, 64.0, 69)]);
        more(&mut rig, "Guess Scale...");
        assert!(
            rig.text("Analysis results for \"Instrument\":").is_some(),
            "{:?}",
            rig.texts
        );
        let five = rows(&rig);
        assert_eq!(five.len(), 5);
        assert_eq!(five[0], "#1 D Major (All in scale, 1 root notes)");
        rig.tap(rig.text("Show more").unwrap());
        rig.frame(vec![]);
        assert_eq!(rows(&rig).len(), 12);
        assert_eq!(rows(&rig)[..5], five[..]);
        rig.tap(rig.text("Show less").unwrap());
        rig.frame(vec![]);
        assert_eq!(rows(&rig), five);
        assert!(rig.all.is_empty());
        rig.tap(rig.text(&five[0]).unwrap());
        assert_eq!(key_lock(&rig), (2, super::stored("Major")));
        assert_eq!(rig.all, [DONE]);
        rig.frame(vec![]);
        assert!(rows(&rig).is_empty(), "the dialog stayed open");
    }

    /// The scale-guess result is on the dialog base: screen dim α 0.6, then the floating panel, from the first frame,
    /// and no popup look (a popup sweeps open over 23 frames in a `text` fill). The animations are on, so a popup would
    /// show: the dim and box are there at once, and no `text` fill ever lies under the rows.
    #[test]
    fn the_scale_guess_result_is_on_the_dialog_base_the_dim_then_the_panel() {
        let mut rig = Rig::new(vec![n(0, 64.0, 62), n(128, 64.0, 66), n(256, 64.0, 69)]);
        rig.select(vec![true; 3]);
        rig.tap(rig.text("More...").unwrap());
        rig.frame(vec![]);
        // The menu is open; from here on the popups animate.
        crate::ui::anim::enable(&rig.ctx);
        rig.tap(rig.text("Guess Scale...").unwrap());
        let colors = theme_by_id(0).colors;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0));
        let (panel_fill, text_fill) = (colors.menu_fill.to_color32(), colors.text.to_color32());
        // The box: centred on the screen, the floating panel's `menuFill`.
        let boxed_in = |rig: &Rig| {
            (rig.painted.iter())
                .find(|&&(r, f)| {
                    f == panel_fill && r.width() > 100.0 && r.center() == screen.center()
                })
                .map(|p| p.0)
        };
        let mut boxed = None;
        // The frame after the tap is the dialog's first; every frame from then on has the base, and no `text` fill wider
        // than 100 (a popup's fill, or its bar once grown) meets the box.
        for n in 0..26 {
            rig.frame(vec![]);
            let b =
                boxed_in(&rig).unwrap_or_else(|| panic!("frame {n}: no box in {:?}", rig.painted));
            boxed = Some(b);
            crate::ui::dialog::assert_base(&rig.painted, &colors, screen, b);
            let popup_fill = (rig.painted.iter())
                .find(|&&(r, f)| f == text_fill && r.width() > 100.0 && r.intersects(b));
            assert_eq!(popup_fill, None, "frame {n}: a popup's fill under the rows");
        }
        let boxed = boxed.unwrap();
        // The title, the rows and `Disable` are in it, drawn at once: no sweep.
        let dialog = |t: &&(String, Rect)| {
            t.0.starts_with("Analysis") || t.0.starts_with('#') || t.0 == "Disable"
        };
        let texts: Vec<_> = rig.texts.iter().filter(dialog).collect();
        assert_eq!(texts.len(), 1 + 5 + 1, "{:?}", rig.texts);
        assert!(texts.iter().all(|(_, r)| boxed.contains_rect(*r)));
    }

    #[test]
    fn guess_scale_needs_three_different_notes() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 72), n(256, 64.0, 67)]);
        rig.select(vec![true, true, false]);
        rig.tap(rig.text("More...").unwrap());
        rig.frame(vec![]);
        rig.tap(rig.text("Guess Scale...").unwrap());
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            rig.text("Please select at least three different notes.")
                .is_some()
        );
        rig.tap(rig.text("OK").unwrap());
        rig.frame(vec![]);
        assert_eq!(rig.text("OK"), None);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn without_an_exact_match_choose_shows_the_closest_and_disable_unlocks_the_scale() {
        let mut rig = Rig::new((0..12).map(|i| n(i * 32, 32.0, 60 + i as u8)).collect());
        // A minor scale, its chord set the untouched Minor default.
        (rig.song.key_root, rig.song.key_scale) = (4, super::stored("Minor Natural"));
        let minor = crate::song::chords::default_keyb(rig.song.key_scale);
        rig.song.file.keyboard.chord_json = minor.0;
        more(&mut rig, "Guess Scale...");
        assert!(rig.text("No exact matches found.").is_some());
        assert!(
            rig.text("Press 'Choose' to manually select from closest options or 'Disable' to deactivate scale locking.")
                .is_some()
        );
        assert!(rows(&rig).is_empty());
        rig.tap(rig.text("Choose").unwrap());
        rig.frame(vec![]);
        assert_eq!(
            rows(&rig)[0],
            "#1 C Major Bebop (4 accidentals, 8 in scale, 1 root notes)"
        );
        rig.tap(rig.text("Disable").unwrap());
        assert_eq!(key_lock(&rig), (4, super::stored("Chromatic")));
        // The untouched chord set follows the scale: Chromatic's default is Major.
        let keyb = &rig.song.file.keyboard;
        assert_eq!(
            (keyb.chord_json.as_str(), keyb.chord_set_name.as_str()),
            (crate::song::DEFAULT_CHORD_JSON, "Major")
        );
        assert_eq!(rig.all, [DONE]);
    }

    // ---- the graph lane ----

    /// In the graph lane's tab on the editor's bottom edge while the lane is closed (centred on the key column's
    /// right edge, x 1.5 U).
    const KNOB: Pos2 = pos2(48.0, 340.0);
    /// The lane at the first show (f 0.5 of H = 349, the editor below its top window margin, which is 0: the ruler
    /// counts): its strip from y 349 − trunc(174.5) = 175, its list and value area under the 4 px gap, 170 tall.
    const STRIP_TOP: f32 = 175.0;
    const LANE_TOP: f32 = 179.0;

    /// The y `v` of the way up the value area.
    fn lane_y(v: f32) -> f32 {
        349.0 - v * (349.0 - LANE_TOP)
    }

    /// A rig with the lane shown by a tap on its knob.
    fn lane_rig(notes: Vec<Event>) -> Rig {
        let mut rig = Rig::new(notes);
        rig.tap(KNOB);
        rig.frame(vec![]);
        assert!(rig.roll.graph.open);
        rig
    }

    /// (velocity, pan) of every note.
    fn values(rig: &Rig) -> Vec<(u8, u8)> {
        rig.events()
            .iter()
            .map(|e| match *e {
                Event::Note { velocity, pan, .. } => (velocity, pan),
                _ => panic!("not a note"),
            })
            .collect()
    }

    /// Presses at the first point, moves to each of the others (one frame each), releases at the last.
    fn stroke(rig: &mut Rig, points: &[Pos2]) {
        rig.frame(vec![In::PointerMoved(points[0])]);
        rig.button(points[0], PointerButton::Primary, true);
        for &p in &points[1..] {
            rig.frame(vec![In::PointerMoved(p)]);
        }
        rig.button(points[points.len() - 1], PointerButton::Primary, false);
    }

    #[test]
    fn the_lane_is_hidden_until_its_knob_is_tapped_and_lists_vel_over_pan() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        assert_eq!(rig.text("Vel"), None);
        // Hidden, the grid reaches the bottom: a tap there adds a note.
        rig.tap(pos2(at(300.0, 70).x, 340.0));
        assert_eq!(rig.events().len(), 2);
        rig.tap(KNOB);
        rig.frame(vec![]);
        let (vel, pan) = (rig.text("Vel").unwrap(), rig.text("Pan").unwrap());
        assert!(vel.x < 48.0 && pan.x < 48.0, "{vel:?} {pan:?}");
        assert!(
            LANE_TOP < vel.y && vel.y < pan.y && pan.y < 349.0,
            "{vel:?} {pan:?}"
        );
        let values = rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("graph"))
            .unwrap()
            .rect;
        // The value area starts at the grid's x (1.5 U + t): the t gap after the list is `background`.
        assert_eq!(
            values,
            Rect::from_min_max(pos2(50.0, LANE_TOP), pos2(1170.0, 349.0))
        );
        rig.tap(pan);
        assert_eq!(rig.roll.graph.item, 1);
        rig.tap(vel);
        assert_eq!(rig.roll.graph.item, 0);
        // The grid ends above the lane, and the tab on the lane's top closes it again.
        rig.tap(pos2(at(500.0, 70).x, 300.0));
        assert_eq!(rig.events().len(), 2);
        rig.tap(pos2(48.0, STRIP_TOP - 10.0));
        rig.frame(vec![]);
        assert_eq!(rig.text("Vel"), None);
        assert!(!rig.roll.graph.open);
        // The first tap's new note; the tap on the lane, which painted nothing, re-syncs.
        assert_eq!(rig.all, [QUIET, PianoRollAction::Resync]);
    }

    #[test]
    fn a_touch_and_each_move_paint_the_notes_under_x_with_no_interpolation() {
        let mut rig = lane_rig(vec![
            n(0, 64.0, 70),
            n(0, 64.0, 66),
            n(128, 64.0, 60),
            n(256, 64.0, 66),
        ]);
        rig.all.clear();
        // Down over the chord halfway up, a move above the lane (y is clamped to it: the top), one far below it
        // over the last note (the bottom): the note between keeps its value. Vel = clamp(trunc(255 v), 1, 255).
        let (chord, last) = (at(10.0, 70).x, at(300.0, 70).x);
        stroke(
            &mut rig,
            &[
                pos2(chord, lane_y(0.5)),
                pos2(chord, 150.0),
                pos2(last, 500.0),
            ],
        );
        assert_eq!(values(&rig), [(255, 127), (255, 127), (204, 127), (1, 127)]);
        assert_eq!(rig.all, [EDITED, EDITED, EDITED, DONE]);
        // The last value painted is the remembered note's V.
        assert_eq!(rig.roll.remembered.velocity, 1);
        // Pan = clamp(trunc(254 v), 0, 254): the middle is 127.
        rig.tap(rig.text("Pan").unwrap());
        let mid = at(140.0, 70).x;
        stroke(&mut rig, &[pos2(mid, lane_y(0.5)), pos2(mid, 0.0)]);
        rig.tap(pos2(last, lane_y(0.5)));
        assert_eq!(values(&rig)[2..], [(204, 254), (1, 127)]);
        assert_eq!(rig.roll.remembered.pan, 254);
    }

    #[test]
    fn with_value_snap_a_lane_paint_lands_on_the_grid_and_off_it_is_continuous() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70)]);
        let pick = |rig: &mut Rig, label: &str| {
            rig.roll.menu = Some(super::super::Menu::Snap);
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig.tap(rig.text(label).expect(label));
        };
        // 0.31 up: Vel 79.05. At 10 % the nearest line is 30 %, 76.5 → 77; the MIDI steps' is k = 39, 39·255/127 =
        // 78.3 → 78, which a MIDI export writes as 39; off it is trunc(79.05) as before.
        let at_vel = |rig: &mut Rig, label: &str| {
            pick(rig, label);
            rig.tap(pos2(at(10.0, 70).x, lane_y(0.31)));
            values(rig)[0].0
        };
        assert_eq!(at_vel(&mut rig, "10 %"), 77);
        assert_eq!(at_vel(&mut rig, "MIDI"), 78);
        assert_eq!(at_vel(&mut rig, "Off"), 79);
        // Pan's own snap: left, centre or right; 0.85 up (215.9) lands on the right.
        rig.tap(rig.text("Pan").unwrap());
        pick(&mut rig, "L/C/R");
        rig.tap(pos2(at(10.0, 70).x, lane_y(0.85)));
        assert_eq!(values(&rig), [(79, 254)]);
    }

    #[test]
    fn while_notes_are_selected_only_they_take_the_paint() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70), n(0, 64.0, 66), n(128, 64.0, 60)]);
        rig.roll.selected = vec![false, true, true];
        rig.frame(vec![]);
        let (chord, next) = (at(10.0, 70).x, at(150.0, 70).x);
        stroke(
            &mut rig,
            &[pos2(chord, lane_y(0.5)), pos2(chord, 0.0), pos2(next, 0.0)],
        );
        assert_eq!(values(&rig), [(204, 127), (255, 127), (255, 127)]);
        assert_eq!(rig.roll.selected, [false, true, true]);
    }

    #[test]
    fn a_gesture_that_changes_nothing_clears_the_selection_and_reports_its_re_sync() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70), n(128, 64.0, 60)]);
        rig.roll.selected = vec![true, false];
        rig.frame(vec![]);
        rig.all.clear();
        // Past the notes: nothing to paint.
        rig.tap(pos2(at(600.0, 70).x, lane_y(0.3)));
        assert_eq!(values(&rig), [(204, 127), (204, 127)]);
        assert_eq!(rig.roll.selected, [false, false]);
        // Its own undo re-syncs.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
    }

    #[test]
    fn a_lane_drag_that_ends_on_the_original_values_closes_its_edit_with_a_re_sync() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70)]);
        rig.tap(rig.text("Pan").unwrap());
        rig.all.clear();
        let x = at(10.0, 70).x;
        // From the centre (127, no change) up to 254 and back to 127.
        stroke(
            &mut rig,
            &[pos2(x, lane_y(0.5)), pos2(x, 0.0), pos2(x, lane_y(0.5))],
        );
        assert_eq!(values(&rig), [(204, 127)]);
        // A touch that changed nothing: its own undo re-syncs, and the app closes the gesture.
        assert_eq!(rig.all, [EDITED, EDITED, PianoRollAction::Resync]);
    }

    #[test]
    fn a_focus_loss_ends_a_lane_drag_and_a_stale_press_paints_nothing() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70), n(128, 64.0, 60)]);
        rig.all.clear();
        let (a, b) = (at(10.0, 70).x, at(150.0, 70).x);
        rig.frame(vec![In::PointerMoved(pos2(a, lane_y(0.5)))]);
        rig.button(pos2(a, lane_y(0.5)), PointerButton::Primary, true);
        rig.frame(vec![In::WindowFocused(false)]);
        assert_eq!(rig.all, [EDITED, DONE]);
        rig.frame(vec![
            In::WindowFocused(true),
            In::PointerMoved(pos2(b, lane_y(0.0))),
        ]);
        rig.frame(vec![In::PointerMoved(pos2(b, lane_y(0.5)))]);
        rig.button(pos2(b, lane_y(0.5)), PointerButton::Primary, false);
        assert_eq!(values(&rig), [(127, 127), (204, 127)]);
        assert_eq!(rig.all, [EDITED, DONE]);
    }

    /// A pinch before any paint ends a touch that changed nothing: its own undo re-syncs, once.
    #[test]
    fn a_pinch_before_any_lane_paint_is_a_touch_that_changed_nothing() {
        let mut rig = lane_rig(vec![n(0, 64.0, 70)]);
        rig.all.clear();
        // Past the note: the touch-down paints nothing.
        let x = at(600.0, 70).x;
        rig.touch(1.0 / 60.0, TouchPhase::Start, pos2(x, lane_y(0.5)));
        rig.second(TouchPhase::Start, at(300.0, 70));
        assert_eq!(rig.all, [PianoRollAction::Resync]);
        rig.second(TouchPhase::End, at(300.0, 70));
        rig.touch(1.0 / 60.0, TouchPhase::End, pos2(x, lane_y(0.5)));
        assert_eq!(rig.all, [PianoRollAction::Resync]);
    }

    #[test]
    fn a_second_finger_puts_a_lane_drag_back_and_nothing_paints_until_the_lift() {
        // The pinch's first finger on the lane paints no more.
        let mut rig = lane_rig(vec![n(0, 64.0, 70)]);
        rig.all.clear();
        let x = at(10.0, 70).x;
        rig.touch(1.0 / 60.0, TouchPhase::Start, pos2(x, lane_y(0.5)));
        rig.touch(1.0 / 60.0, TouchPhase::Move, pos2(x, lane_y(1.0)));
        assert_eq!(values(&rig), [(255, 127)]);
        // The second finger lands on the grid: the drag ends with the note as it was.
        rig.second(TouchPhase::Start, at(300.0, 70));
        assert_eq!(values(&rig), [(204, 127)]);
        rig.touch(1.0 / 60.0, TouchPhase::Move, pos2(x, lane_y(0.2)));
        rig.second(TouchPhase::End, at(300.0, 70));
        rig.touch(1.0 / 60.0, TouchPhase::Move, pos2(x, lane_y(0.9)));
        rig.touch(1.0 / 60.0, TouchPhase::End, pos2(x, lane_y(0.9)));
        assert_eq!(values(&rig), [(204, 127)]);
        // Put back, the touch changed nothing: its own undo re-syncs.
        assert_eq!(rig.all, [EDITED, EDITED, PianoRollAction::Resync]);
        // The next touch paints again.
        rig.touch(1.0 / 60.0, TouchPhase::Start, pos2(x, lane_y(0.5)));
        rig.touch(1.0 / 60.0, TouchPhase::End, pos2(x, lane_y(0.5)));
        assert_eq!(values(&rig), [(127, 127)]);
    }

    #[test]
    fn vel_draws_a_stem_and_a_cap_per_note_in_its_tint() {
        let mut rig = lane_rig(vec![n(128, 64.0, 70)]);
        rig.frame(vec![]);
        // Velocity 204 in the 170 tall area: the top at 349 − 170·0.8 = 213; tick 128 at x 50 + 64.
        let shapes = &rig.rects;
        assert!(
            shapes.contains(&Rect::from_min_max(pos2(114.0, 213.0), pos2(118.0, 349.0))),
            "the stem: {shapes:?}"
        );
        assert!(
            shapes.contains(&Rect::from_min_max(pos2(114.0, 213.0), pos2(146.0, 215.0))),
            "the cap: {shapes:?}"
        );
    }

    #[test]
    fn ten_guides_show_at_0_to_90_percent_the_first_on_the_row_above_the_value_area() {
        // The 0 % guide shows too. Each is the 1 px row ending at round(top + k·h/10), so the 50 % one is the Pan
        // stems' centre, lane.y − 1 + h/2.
        let rig = lane_rig(vec![]);
        let guide = egui::Color32::from_black_alpha(51);
        let guides: Vec<f32> = (rig.visible.iter())
            .filter(|(r, f)| *f == guide && r.height() > 0.0 && r.width() > 1000.0)
            .map(|(r, _)| r.bottom())
            .collect();
        let want: Vec<f32> = (0..10).map(|k| LANE_TOP + k as f32 * 17.0).collect();
        assert_eq!(guides, want);
    }

    #[test]
    fn the_lanes_beat_lines_are_the_grids_marks() {
        // The grid's beat lines, in `background`: 4 px at a bar, 2 at a beat, 1 finer, at least 0.6 U apart: at 64
        // points a beat, every half beat; w px from ⌊w/2⌋ px left of the mark, clipped to the value area from x 50.
        let rig = lane_rig(vec![]);
        let bg = theme_by_id(0).colors.background.to_color32();
        let lines: Vec<(f32, f32)> = (rig.visible.iter())
            .filter(|(r, f)| {
                *f == bg
                    && r.y_range() == Rangef::new(LANE_TOP, 349.0)
                    && (0.0..=4.0).contains(&r.width())
                    && r.width() > 0.0
                    && r.right() < 330.0
            })
            .map(|(r, _)| (r.left(), r.width()))
            .collect();
        assert_eq!(
            lines,
            [
                (50.0, 2.0),
                (82.0, 1.0),
                (113.0, 2.0),
                (146.0, 1.0),
                (177.0, 2.0),
                (210.0, 1.0),
                (241.0, 2.0),
                (274.0, 1.0),
                (304.0, 4.0)
            ]
        );
    }

    // ---- the ruler's gestures and the region menu ----

    /// On the ruler, over content tick `tick`; tick 0 just inside it, right of the snap cell (which ends at x 48).
    fn ruler(tick: f64) -> Pos2 {
        pos2(at(tick, 60).x.max(48.5), 24.0)
    }

    /// A long press over `from` (held 0.3 s), dragged to `to` and released: the range (the song's play loop).
    fn select(rig: &mut Rig, from: f64, to: f64) {
        rig.frame(vec![In::PointerMoved(ruler(from))]);
        rig.button(ruler(from), PointerButton::Primary, true);
        rig.frame_after(0.31, vec![]);
        rig.frame(vec![In::PointerMoved(ruler(to))]);
        rig.button(ruler(to), PointerButton::Primary, false);
        rig.frame(vec![]);
    }

    /// Selects `from..to` and taps the `+` under the released edge: the region menu opens.
    fn region(rig: &mut Rig, from: f64, to: f64) {
        select(rig, from, to);
        rig.tap(rig.text("+").expect("the + button"));
        rig.frame(vec![]);
    }

    fn starts(rig: &Rig) -> Vec<u32> {
        spans(rig.events()).iter().map(|s| s.0).collect()
    }

    #[test]
    fn a_range_on_the_ruler_is_the_play_loop_and_its_menu_has_the_scale_tools_last() {
        let notes = vec![
            n(0, 64.0, 60),
            n(128, 64.0, 62),
            n(256, 64.0, 64),
            n(384, 64.0, 65),
        ];
        let mut rig = Rig::new(notes);
        // Made from its end, clear of the play box's touch area at beat 0.
        select(&mut rig, 256.0, 128.0);
        // The clip starts at song tick 0: content ticks are song ticks.
        assert_eq!(rig.song.play_loop, Some((128, 256)));
        assert_eq!(rig.text("Deselect"), None, "the menu waits for the +");
        rig.tap(rig.text("+").unwrap());
        rig.frame(vec![]);
        let rect = |label: &str| {
            (rig.texts.iter())
                .find(|(t, _)| t == label)
                .map(|(_, r)| *r)
                .expect(label)
        };
        let (trim, snap, guess) = (
            rect("Trim selection"),
            rect("Snap to Scale"),
            rect("Guess Scale..."),
        );
        // Column 0, below `Trim selection`.
        assert_eq!((snap.left(), guess.left()), (trim.left(), trim.left()));
        assert!(trim.top() < snap.top() && snap.top() < guess.top());
        assert!(rig.all.is_empty());
        rig.tap(rig.text("Insert space").unwrap());
        assert_eq!(starts(&rig), [0, 256, 384, 512]);
        assert_eq!(rig.song.channels[1].lanes[0].clips[0].loop_len_beats, 8.0);
        assert_eq!(rig.all, [DONE]);
        assert_eq!(rig.song.play_loop, None);
    }

    #[test]
    fn the_range_follows_the_clips_place_in_the_song() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60)]);
        // The clip a bar into the song, two beats into its content: content tick 0 plays at song tick 256.
        let clip = &mut rig.song.channels[1].lanes[0].clips[0];
        (clip.start_tick, clip.offset_beats) = (512, 2.0);
        rig.frame(vec![]);
        select(&mut rig, 256.0, 128.0);
        assert_eq!(rig.song.play_loop, Some((384, 512)));
    }

    #[test]
    fn a_range_wholly_before_the_song_start_is_no_play_loop() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60)]);
        // Five beats into its one-bar content: content tick 0 plays at song tick −128 (Preview's span).
        rig.song.channels[1].lanes[0].clips[0].offset_beats = 5.0;
        rig.frame(vec![]);
        // Song ticks −128..0: empty from tick 0, so no range.
        select(&mut rig, 0.0, 128.0);
        assert_eq!(rig.song.play_loop, None);
    }

    #[test]
    fn snap_to_scale_on_a_range_snaps_only_the_notes_that_start_in_it() {
        let mut rig = Rig::new(vec![n(0, 64.0, 61), n(128, 64.0, 61), n(256, 64.0, 61)]);
        (rig.song.key_root, rig.song.key_scale) = (0, super::stored("Major"));
        region(&mut rig, 256.0, 128.0);
        rig.tap(rig.text("Snap to Scale").unwrap());
        assert_eq!(keys(&rig), [61, 60, 61]);
        // It re-syncs whatever it changed.
        assert_eq!(rig.all, [PianoRollAction::Resync]);
        assert_eq!(rig.song.play_loop, None);
    }

    #[test]
    fn guess_scale_on_a_range_analyses_the_notes_that_start_in_it() {
        // C E G in all, C E in the range.
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 64), n(256, 64.0, 67)]);
        rig.knob_away();
        region(&mut rig, 0.0, 256.0);
        rig.tap(rig.text("Guess Scale...").unwrap());
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            rig.text("Please select at least three different notes.")
                .is_some()
        );
        rig.tap(rig.text("OK").unwrap());
        region(&mut rig, 0.0, 384.0);
        rig.tap(rig.text("Guess Scale...").unwrap());
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            rig.text("Analysis results for \"Instrument\":").is_some(),
            "{:?}",
            rig.texts
        );
        assert!(rig.all.is_empty());
    }

    #[test]
    fn the_playhead_items_follow_the_transports_playhead_in_the_clip() {
        // Inside the range, or past the clip: both are off. The range is made from its end, clear of the play box's
        // touch area.
        for playhead in [160, 5000] {
            let mut rig = Rig::new(vec![n(128, 64.0, 62)]);
            rig.opts.playhead = playhead;
            region(&mut rig, 384.0, 128.0);
            rig.tap(rig.text("Copy to playhead").unwrap());
            rig.tap(rig.text("Move to playhead").unwrap());
            assert!(rig.all.is_empty(), "{playhead:?}");
        }
        // Stopped at 0: the copy is inserted there, pushing the source on.
        let mut rig = Rig::new(vec![n(128, 64.0, 62)]);
        rig.opts.playhead = 0;
        region(&mut rig, 256.0, 128.0);
        rig.tap(rig.text("Copy to playhead").unwrap());
        assert_eq!(starts(&rig), [0, 256]);
        assert_eq!(rig.all, [DONE]);
    }

    #[test]
    fn a_tap_on_the_ruler_asks_for_the_playhead_on_the_snap_grid_and_the_handle_shows_it() {
        let mut rig = Rig::new(vec![]);
        rig.tap(ruler(200.0));
        // Auto snaps to steps (32 ticks) at this zoom; the clip's content tick 0 is song tick 0.
        assert_eq!(rig.all, [PianoRollAction::SetPlayhead(192)]);
        // Stopped there, the handle rests on it.
        rig.opts.playhead = 192;
        rig.frame(vec![]);
        assert_eq!(rig.handle, Some(r3(ruler(192.0).x)));
    }

    #[test]
    fn a_tap_on_an_offset_clips_ruler_seeks_where_that_content_plays() {
        // At song tick 512, two beats into its one-bar content: content tick 384 (past the play box's touch area at
        // content tick 0) plays at 512 + 128.
        let mut rig = Rig::new(vec![]);
        let clip = &mut rig.song.channels[1].lanes[0].clips[0];
        (clip.start_tick, clip.offset_beats) = (512, 2.0);
        rig.frame(vec![]);
        rig.tap(ruler(384.0));
        assert_eq!(rig.all, [PianoRollAction::SetPlayhead(640)]);
        // Read back, the handle is where the tap was.
        rig.opts.playhead = 640;
        rig.frame(vec![]);
        assert_eq!(rig.handle, Some(r3(ruler(384.0).x)));
    }

    #[test]
    fn a_region_edit_drops_the_note_selection() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60), n(128, 64.0, 62)]);
        rig.roll.selected = vec![true, true];
        rig.knob_away();
        region(&mut rig, 0.0, 128.0);
        rig.tap(rig.text("Delete space").unwrap());
        assert_eq!(starts(&rig), [0]);
        assert!(rig.roll.selected.iter().all(|s| !s));
    }

    // ---- the keymap's editing and zoom actions ----

    impl Rig {
        /// A press of `key` with `mods` and its release, in one frame.
        fn key(&mut self, key: egui::Key, mods: Modifiers) {
            self.frame(crate::ui::edit_keys::tests::tap(key, mods).into());
        }
    }

    const CMD: Modifiers = Modifiers::COMMAND;

    /// The edits of the last frame: one final one per mutating action.
    fn done(rig: &Rig) -> usize {
        rig.last.iter().filter(|a| **a == DONE).count()
    }

    #[test]
    fn ctrl_c_copies_the_selection_and_ctrl_v_pastes_it_at_the_touched_notes_cell() {
        let notes = vec![n(0, 128.0, 60), n(128, 64.0, 64), n(512, 128.0, 70)];
        let mut rig = Rig::new(notes);
        rig.select(vec![true, true, false]);
        rig.key(egui::Key::C, CMD);
        assert_eq!(rig.last, []);
        assert_eq!(spans(rig.events()).len(), 3, "copy changes nothing");
        // Touching a note sets the grid cursor to the touched beat and key: here tick 544 (17 snap steps), key 70.
        rig.tap(at(544.0, 70));
        rig.key(egui::Key::V, CMD);
        assert_eq!(done(&rig), 1, "one undo step");
        // The earliest note lands on the cursor cell; the others keep their offsets in time and key.
        assert_eq!(
            spans(rig.events()),
            [
                (0, 128.0, 60),
                (128, 64.0, 64),
                (512, 128.0, 70),
                (544, 128.0, 70),
                (672, 64.0, 74)
            ]
        );
        // The pasted notes are the selection, without the floating row.
        assert_eq!(rig.roll.selected, [false, false, false, true, true]);
        assert_eq!(rig.roll.row.rect(), None);
    }

    #[test]
    fn ctrl_x_cuts_and_delete_deletes_the_selection_each_in_one_undo_step() {
        let mut rig = Rig::new(vec![n(0, 128.0, 60), n(128, 64.0, 64), n(256, 64.0, 65)]);
        rig.select(vec![true, false, false]);
        rig.key(egui::Key::X, CMD);
        assert_eq!(done(&rig), 1);
        assert_eq!(spans(rig.events()), [(128, 64.0, 64), (256, 64.0, 65)]);
        rig.select(vec![false, true]);
        rig.key(egui::Key::Delete, Modifiers::NONE);
        assert_eq!(done(&rig), 1);
        assert_eq!(spans(rig.events()), [(128, 64.0, 64)]);
        // Nothing selected: nothing to delete, no step.
        rig.key(egui::Key::Delete, Modifiers::NONE);
        assert_eq!(rig.last, []);
        // The cut note is on the clipboard: pasted at the cursor, which starts at tick 0, key 60.
        rig.key(egui::Key::V, CMD);
        assert_eq!(spans(rig.events()), [(0, 128.0, 60), (128, 64.0, 64)]);
    }

    #[test]
    fn paste_rounds_an_off_grid_cursor_to_the_snap_unit() {
        let mut rig = Rig::new(vec![n(0, 128.0, 60), n(512, 128.0, 70)]);
        rig.select(vec![true, false]);
        rig.key(egui::Key::C, CMD);
        // Touched at tick 555: the snap unit here is a step (32 ticks), so the paste lands on 544.
        rig.tap(at(555.0, 70));
        rig.key(egui::Key::V, CMD);
        assert_eq!(
            spans(rig.events()),
            [(0, 128.0, 60), (512, 128.0, 70), (544, 128.0, 70)]
        );
    }

    #[test]
    fn paste_with_an_empty_clipboard_does_nothing() {
        let mut rig = Rig::new(vec![n(0, 128.0, 60)]);
        rig.key(egui::Key::V, CMD);
        assert_eq!(rig.last, []);
        assert_eq!(spans(rig.events()), [(0, 128.0, 60)]);
    }

    #[test]
    fn ctrl_a_selects_every_note_and_escape_deselects_without_an_edit() {
        let mut rig = Rig::new(vec![n(0, 128.0, 60), n(128, 64.0, 64)]);
        rig.key(egui::Key::A, CMD);
        assert_eq!(rig.roll.selected, [true, true]);
        assert_eq!(rig.roll.row.rect(), None, "the handles' row stays hidden");
        rig.key(egui::Key::Escape, Modifiers::NONE);
        assert_eq!(rig.roll.selected, [false, false]);
        assert_eq!(rig.all, []);
    }

    #[test]
    fn enter_adds_a_note_at_the_cursor_cell_in_one_undo_step() {
        let mut rig = Rig::new(vec![n(512, 64.0, 67)]);
        rig.key(egui::Key::Enter, Modifiers::NONE);
        assert_eq!((done(&rig), rig.last.contains(&QUIET)), (0, true));
        // The cursor starts at tick 0, key 60; the note takes the remembered length (a beat).
        assert_eq!(spans(rig.events()), [(0, 128.0, 60), (512, 64.0, 67)]);
        // A touched note moves the cursor there; its cell holds that note, so Enter adds nothing.
        rig.tap(at(520.0, 67));
        rig.key(egui::Key::Enter, Modifiers::NONE);
        assert_eq!(rig.last, []);
        assert_eq!(spans(rig.events()), [(0, 128.0, 60), (512, 64.0, 67)]);
        // Deleted, the cell is empty again: Enter adds a note there, the touched one's length.
        rig.select(vec![false, true]);
        rig.key(egui::Key::Delete, Modifiers::NONE);
        rig.key(egui::Key::Enter, Modifiers::NONE);
        assert_eq!((done(&rig), rig.last.contains(&QUIET)), (0, true));
        assert_eq!(spans(rig.events()), [(0, 128.0, 60), (512, 64.0, 67)]);
    }

    #[test]
    fn enter_checks_the_same_floored_visible_cell_that_it_creates_in() {
        for (occupied, expected) in [(64, 1), (128, 2)] {
            let mut rig = Rig::new(vec![n(occupied, 32.0, 60)]);
            rig.roll.cursor = (113.0, 60);
            rig.key(egui::Key::Enter, Modifiers::NONE);
            assert_eq!(rig.events().len(), expected);
            assert_eq!(spans(rig.events())[0].0, 64);
        }
    }

    #[test]
    fn escape_closes_an_open_popup_before_it_deselects() {
        let mut rig = Rig::new(vec![n(0, 128.0, 60)]);
        rig.key(egui::Key::A, CMD);
        rig.roll.menu = Some(super::super::Menu::Snap);
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig.key(egui::Key::Escape, Modifiers::NONE);
        assert!(rig.roll.menu.is_none());
        assert_eq!(rig.roll.selected, [true], "the popup took the Escape");
    }

    #[test]
    fn ctrl_held_makes_a_touch_on_a_note_a_marquee_instead_of_a_move() {
        // The keymap's `editorSelect`.
        let notes = vec![n(0, 128.0, 60), n(256, 64.0, 62)];
        let mut rig = Rig::new(notes.clone());
        rig.modifiers = CMD;
        rig.drag(at(64.0, 60), at(300.0, 62));
        assert_eq!(rig.events(), notes);
        assert_eq!(rig.roll.selected, [true, true]);
        assert_eq!(rig.all, []);
    }

    // ---- a touch the roll stops seeing ----

    const OFF_71: PianoRollAction = PianoRollAction::Note {
        key: 71,
        velocity: None,
    };

    /// Key 70's note at tick 128 pressed and dragged up a row: key 71 sounds, one ongoing edit; still held there.
    fn drag_up_a_row(rig: &mut Rig) -> Pos2 {
        let from = at(140.0, 70);
        rig.frame(vec![In::PointerMoved(from)]);
        rig.button(from, PointerButton::Primary, true);
        // Out of the 0.5 U calm zone, 1.25 rows up.
        let to = from - vec2(0.0, 20.0);
        rig.frame(vec![In::PointerMoved(to)]);
        assert_eq!(rig.sounds, [(71, Some(0.7))]);
        assert_eq!(rig.all, [EDITED]);
        to
    }

    #[test]
    fn a_roll_hidden_mid_drag_stops_the_dragged_key_and_closes_the_move_where_it_is() {
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let to = drag_up_a_row(&mut rig);
        assert_eq!(rig.roll.hide(), [OFF_71, DONE]);
        assert_eq!(spans(rig.events()), [(128, 64.0, 71)]);
        // Shown again, the release ends nothing more.
        rig.release(to);
        rig.frame(vec![]);
        assert_eq!(rig.sounds, [(71, Some(0.7))]);
        assert_eq!(rig.all, [EDITED]);
        assert_eq!(spans(rig.events()), [(128, 64.0, 71)]);
        assert!(rig.roll.hide().is_empty());
    }

    #[test]
    fn a_roll_hidden_mid_lane_touch_ends_it_as_its_release_would() {
        // One that changed nothing re-syncs and clears the selection; one that painted is final.
        let x = at(10.0, 70).x;
        let mut rig = lane_rig(vec![n(0, 64.0, 70)]);
        rig.roll.selected = vec![true];
        rig.frame(vec![]);
        // Painted up, then back to its 204.
        let v204 = lane_y(204.5 / 255.0);
        rig.frame(vec![In::PointerMoved(pos2(x, v204))]);
        rig.button(pos2(x, v204), PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(pos2(x, lane_y(1.0)))]);
        rig.frame(vec![In::PointerMoved(pos2(x, v204))]);
        assert_eq!(values(&rig), [(204, 127)]);
        assert_eq!(rig.roll.hide(), [PianoRollAction::Resync]);
        assert_eq!(rig.roll.selected, [false]);
        rig.release(pos2(x, v204));
        assert!(rig.roll.hide().is_empty());
        // Painted and still changed: the final edit.
        rig.frame(vec![In::PointerMoved(pos2(x, v204))]);
        rig.button(pos2(x, v204), PointerButton::Primary, true);
        rig.frame(vec![In::PointerMoved(pos2(x, lane_y(1.0)))]);
        assert_eq!(rig.roll.hide(), [DONE]);
        rig.release(pos2(x, lane_y(1.0)));
        assert_eq!(values(&rig), [(255, 127)]);
    }

    #[test]
    fn a_hidden_roll_lets_go_of_its_key_column_key_and_of_a_new_notes_gate() {
        let mut rig = Rig::new(vec![]);
        let c5 = pos2(20.0, at(0.0, 72).y);
        rig.frame(vec![In::PointerMoved(c5)]);
        rig.button(c5, PointerButton::Primary, true);
        let off = |key| PianoRollAction::Note {
            key,
            velocity: None,
        };
        assert_eq!(rig.roll.hide(), [off(72)]);
        rig.release(c5);
        assert_eq!(rig.sounds, [(72, Some(crate::app::KEY_VELOCITY))]);
        // A new note sounds for its 150 ms gate: hidden within it, it stops at once, and not again.
        rig.tap(at(256.0, 60));
        assert_eq!(rig.sounds[1..], [(60, Some(1.0))]);
        assert_eq!(rig.roll.hide(), [off(60)]);
        rig.frame_after(0.2, vec![]);
        assert_eq!(rig.sounds.len(), 2, "{:?}", rig.sounds);
    }

    #[test]
    fn opening_another_clip_mid_drag_stops_the_key_and_closes_the_move() {
        // `Select clip` on another clip of the channel's notes lane while a note drags.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        let lane = &mut song.channels[1].lanes[0];
        lane.add_clip(0, 4.0).events = vec![n(128, 64.0, 70)];
        let other = lane.add_clip(2048, 4.0).id;
        let mut rig = Rig::with_song(song);
        let to = drag_up_a_row(&mut rig);
        rig.roll.open(0, other);
        rig.frame(vec![]);
        assert_eq!(rig.sounds, [(71, Some(0.7)), (71, None)]);
        assert_eq!(rig.all, [EDITED, DONE]);
        rig.release(to);
        assert_eq!(rig.sounds.len(), 2);
        assert_eq!(rig.all, [EDITED, DONE]);
        assert_eq!(spans(rig.events()), [(128, 64.0, 71)]);
    }

    #[test]
    fn an_outside_change_mid_drag_stops_the_dragged_key() {
        // Item 2a: an undo mid note-drag dropped the gesture with its key still sounding.
        let mut rig = Rig::new(vec![n(128, 64.0, 70)]);
        let to = drag_up_a_row(&mut rig);
        rig.song.channels[1].lanes[0].clips[0].events = vec![n(128, 64.0, 70)];
        rig.frame(vec![]);
        assert_eq!(rig.sounds, [(71, Some(0.7)), (71, None)]);
        rig.release(to);
        assert_eq!(rig.sounds.len(), 2);
        assert_eq!(rig.all, [EDITED]);
        assert_eq!(spans(rig.events()), [(128, 64.0, 70)]);
    }

    #[test]
    fn a_key_column_press_held_over_a_focus_loss_stops_its_key_for_good() {
        // Item 2b: as the keyboard's, the drum pads' and the chord keys'.
        let mut rig = Rig::new(vec![]);
        let c5 = pos2(20.0, at(0.0, 72).y);
        rig.frame(vec![In::PointerMoved(c5)]);
        rig.button(c5, PointerButton::Primary, true);
        rig.frame(vec![In::WindowFocused(false)]);
        let velocity = Some(crate::app::KEY_VELOCITY);
        assert_eq!(rig.sounds, [(72, velocity), (72, None)]);
        // egui keeps the button down: the stale press plays nothing, wherever it goes.
        rig.frame(vec![
            In::WindowFocused(true),
            In::PointerMoved(c5 + vec2(0.0, 16.0)),
        ]);
        rig.frame(vec![]);
        rig.release(c5 + vec2(0.0, 16.0));
        assert_eq!(rig.sounds.len(), 2, "{:?}", rig.sounds);
        // The next press plays again.
        rig.frame(vec![In::PointerMoved(c5)]);
        rig.button(c5, PointerButton::Primary, true);
        assert_eq!(rig.sounds[2..], [(72, velocity)]);
    }

    // ---- touch `Paste` on empty grid ----

    /// Two notes on the note clipboard: key 60 at tick 0 and key 62 a half beat later.
    fn copy_two(rig: &Rig) {
        super::super::set_note_clipboard(&rig.ctx, vec![n(0, 64.0, 60), n(64, 64.0, 62)]);
    }

    #[test]
    fn a_long_press_lifted_in_place_on_empty_grid_shows_paste_while_notes_are_copied() {
        let mut rig = Rig::new(vec![n(0, 64.0, 60)]);
        let finger = at(384.0, 66);
        rig.hold(finger);
        rig.release(finger);
        rig.frame(vec![]);
        assert_eq!(rig.text("Paste"), None, "nothing copied");
        copy_two(&rig);
        rig.hold(finger);
        assert!(rig.text("Paste").is_some(), "Paste appears before lifting");
        assert_eq!(spans(rig.events()), [(0, 64.0, 60)]);
        rig.release(finger);
        rig.frame(vec![]);
        // The `Paste` menu remains over the held row until tapped.
        let paste = rig.text("Paste").expect("the Paste button");
        assert!(paste.y < finger.y, "{paste:?}");
        // Its tap lands the earliest note on the held cell's snapped beat, the other at its offsets: one final edit,
        // selected.
        rig.all.clear();
        rig.tap(paste);
        assert_eq!(
            spans(rig.events()),
            [(0, 64.0, 60), (384, 64.0, 66), (448, 64.0, 68)]
        );
        assert_eq!(rig.roll.selected, [false, true, true]);
        assert_eq!(rig.all, [DONE]);
        rig.frame(vec![]);
        assert_eq!(rig.text("Paste"), None);
    }

    #[test]
    fn small_hold_jitter_keeps_only_the_paste_bubble() {
        let mut rig = Rig::new(vec![n(128, 64.0, 66)]);
        copy_two(&rig);
        rig.hold(at(120.0, 66));
        rig.frame(vec![In::PointerMoved(at(132.0, 66))]);
        rig.release(at(132.0, 66));
        rig.frame(vec![]);
        assert!(rig.text("Paste").is_some());
        for label in ["Copy", "Delete", "Snap", "More..."] {
            assert_eq!(rig.text(label), None, "{label} must not accompany Paste");
        }
        assert_eq!(spans(rig.events()), [(128, 64.0, 66)]);
        assert!(rig.all.is_empty());
    }

    #[test]
    fn a_right_click_on_empty_grid_shows_paste_and_any_other_press_hides_it() {
        let mut rig = Rig::new(vec![]);
        right_click(&mut rig, at(384.0, 66));
        rig.frame(vec![]);
        assert_eq!(rig.text("Paste"), None, "nothing copied");
        copy_two(&rig);
        right_click(&mut rig, at(384.0, 66));
        rig.frame(vec![]);
        assert!(rig.text("Paste").is_some());
        rig.tap(at(768.0, 60));
        rig.frame(vec![]);
        assert_eq!(rig.text("Paste"), None);
        assert_eq!(
            spans(rig.events()),
            [(768, 128.0, 60)],
            "the tap's own note"
        );
        // A long press that dragged is a marquee only.
        rig.hold(at(384.0, 66));
        rig.frame(vec![In::PointerMoved(at(640.0, 64))]);
        rig.release(at(640.0, 64));
        rig.frame(vec![]);
        assert_eq!(rig.text("Paste"), None);
    }

    #[test]
    fn a_long_press_held_over_a_focus_loss_shows_no_paste() {
        for release_with_focus_loss in [false, true] {
            let mut rig = Rig::new(vec![]);
            copy_two(&rig);
            let at = at(384.0, 66);
            rig.hold(at);
            assert!(rig.text("Paste").is_some());
            let mut events = vec![In::WindowFocused(false)];
            if release_with_focus_loss {
                events.push(In::PointerButton {
                    pos: at,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            rig.frame(events);
            assert_eq!(rig.text("Paste"), None);
            rig.frame(vec![In::WindowFocused(true)]);
            rig.release(at);
            rig.frame(vec![]);
            assert_eq!(rig.text("Paste"), None);
        }
    }
}

// ---- the grid background ----

mod grid {
    use eframe::egui::{Color32, Context, LayerId, RawInput, Rect, Shape, pos2};

    use super::n;
    use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
    use crate::song::scales::{SCALES, scale_to_stored};
    use crate::song::{ChannelKind, Song, SynthKind, TimeSig};
    use crate::theme::{Rgba, theme_by_id};
    use crate::ui::UiCtx;
    use crate::ui::piano_roll::notes::Geom;
    use crate::ui::piano_roll::{PianoRoll, Rows, paint_grid, paint_notes};

    const RED: Rgba = Rgba::rgb(1.0, 0.0, 0.0);

    /// What `paint` paints at ppp `ppp` on a grid from (1.5 U, 1.5 U) to (1170, 349), 2 U a beat and 0.5 U a row from
    /// tick 0 at its left edge, key 72's row on top: its filled rects, the grid, U and the geometry.
    fn painted(
        ppp: f32,
        paint: &dyn Fn(&eframe::egui::Painter, Rect, &Geom, &UiCtx),
    ) -> (Vec<(Rect, Color32)>, Rect, f32, Geom) {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        let u = m.u(1.0);
        let theme = theme_by_id(0);
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let grid = Rect::from_min_max(pos2(1.5 * u, 1.5 * u), pos2(1170.0, 349.0));
        let g = Geom {
            origin: pos2(grid.left(), grid.top() - 55.0 * 0.5 * u),
            beat_w: 2.0 * u,
            row_h: 0.5 * u,
        };
        let ctx = Context::default();
        let out = ctx.run(RawInput::default(), |ctx| {
            paint(&ctx.layer_painter(LayerId::background()), grid, &g, &cx);
        });
        let rects = (out.shapes.into_iter())
            .filter_map(|s| match s.shape {
                Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect();
        (rects, grid, u, g)
    }

    /// C major light rows from C4 (48) to C6 (72), key 64 lit, root C.
    fn rows() -> Rows {
        let white = |k: usize| [0, 2, 4, 5, 7, 9, 11].contains(&(k % 12));
        Rows {
            light: std::array::from_fn(|k| (48..=72).contains(&k) && white(k)),
            lit: std::array::from_fn(|k| k == 64),
            root: 0,
        }
    }

    fn background(ppp: f32) -> (Vec<(Rect, Color32)>, Rect, f32, Geom) {
        painted(ppp, &|p, grid, g, cx| {
            paint_grid(p, grid, g, &rows(), RED, TimeSig::default(), cx)
        })
    }

    #[test]
    fn the_beat_marks_are_the_shared_adaptive_ones_4_px_bars_2_px_beats_1_px_halves_at_ppp_1_and_2()
    {
        let bg = theme_by_id(0).colors.background.to_color32();
        for ppp in [1.0, 2.0] {
            let (rects, grid, u, _) = background(ppp);
            let px = 1.0 / ppp;
            // The vertical `background` lines down the whole grid: (left, width in px).
            let marks: Vec<(f32, f32)> = (rects.iter())
                .filter(|(r, fill)| *fill == bg && r.y_range() == grid.y_range())
                .map(|(r, _)| (r.left(), r.width() * ppp))
                .collect();
            // At 2 U a beat the finest level 0.6 U apart is the half beat (1 U); a bar is 4 beats.
            let want: Vec<(f32, f32)> = (0..)
                .map(|i| (i, grid.left() + i as f32 * u))
                .take_while(|&(_, x)| x <= grid.right())
                .map(|(i, x)| {
                    let w = match i {
                        _ if i % 8 == 0 => 4.0,
                        _ if i % 2 == 0 => 2.0,
                        _ => 1.0,
                    };
                    (x - (w / 2.0f32).floor() * px, w)
                })
                .collect();
            assert_eq!(marks, want, "ppp {ppp}");
        }
    }

    #[test]
    fn rows_are_control_fill_light_rows_module_fill_each_with_its_lines_lit_and_root_rows_and_no_beat_bands()
     {
        let colors = theme_by_id(0).colors;
        let (control, module, bg) = (
            colors.control_fill.to_color32(),
            colors.module_fill.to_color32(),
            colors.background.to_color32(),
        );
        for ppp in [1.0, 2.0] {
            let (rects, grid, _, g) = background(ppp);
            let px = 1.0 / ppp;
            // 1: `controlFill` over the key rows (all of the grid here).
            assert_eq!(rects[0], (grid, control), "ppp {ppp}");
            let row = |k: u8| Rect::from_x_y_ranges(grid.x_range(), g.y(k)..=g.y(k) + g.row_h);
            let shown: Vec<u8> = (0..=127)
                .filter(|&k| row(k).top() < grid.bottom() && row(k).bottom() > grid.top())
                .collect();
            // 2: whole rows, never beat bands.
            let light: Vec<Rect> = (rects.iter())
                .filter(|(_, fill)| *fill == module)
                .map(|(r, _)| *r)
                .collect();
            let want: Vec<Rect> = (shown.iter().rev())
                .filter(|&&k| rows().light[usize::from(k)])
                .map(|&k| row(k))
                .collect();
            assert_eq!(light, want, "ppp {ppp}");
            // 3: a 1 px `background` α 0.3 line at each row's top and bottom.
            let line = bg.gamma_multiply(0.3);
            let lines = rects
                .iter()
                .filter(|(r, fill)| *fill == line && r.height() == px);
            assert_eq!(lines.count(), 2 * shown.len(), "ppp {ppp}");
            // 4: the lit row, the channel colour α 0.1.
            let lit = RED.to_color32().gamma_multiply(0.1);
            let lit: Vec<Rect> = (rects.iter())
                .filter(|(_, f)| *f == lit)
                .map(|(r, _)| *r)
                .collect();
            assert_eq!(lit, [row(64)], "ppp {ppp}");
            // 6: C rows, a 2 px `background` line at trunc(row bottom) − 2 px across the grid.
            let roots: Vec<f32> = (rects.iter())
                .filter(|(r, fill)| {
                    *fill == bg && r.height() == 2.0 * px && r.x_range() == grid.x_range()
                })
                .map(|(r, _)| r.top())
                .collect();
            let want: Vec<f32> = (shown.iter().rev())
                .filter(|&&k| k % 12 == 0)
                .map(|&k| ((g.y(k) + g.row_h) * ppp).floor() / ppp - 2.0 * px)
                .collect();
            assert_eq!(roots, want, "ppp {ppp}");
            assert!(!want.is_empty());
        }
    }

    #[test]
    fn ghosts_then_black_0_15_past_the_loop_end_then_the_notes() {
        let past = Rect::from_min_max(pos2(304.0, 48.0), pos2(1170.0, 349.0));
        let events = [n(0, 128.0, 60)];
        let (rects, ..) = painted(1.0, &|p, _, g, cx| {
            paint_notes(p, g, &events, &[false], &[], Some(past), RED, cx);
        });
        assert_eq!(
            rects.iter().map(|r| r.1).collect::<Vec<_>>(),
            [Color32::from_black_alpha(38), RED.to_color32()]
        );
        assert_eq!(rects[0].0, past);
    }

    #[test]
    fn light_rows_are_the_project_scale_chromatic_as_c_major_inside_the_channels_instrument_ranges()
    {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let roll = PianoRoll::for_channel(&song, 1);
        let light = |song: &Song| {
            let rows = roll.rows(song, &[], 0);
            (0..128).filter(|&k| rows.light[k]).collect::<Vec<usize>>()
        };
        // Chromatic: the white keys, across the full 0–127 range.
        let white: Vec<usize> = (0..128)
            .filter(|k| [0, 2, 4, 5, 7, 9, 11].contains(&(k % 12)))
            .collect();
        assert_eq!(light(&song), white);
        assert_eq!(roll.rows(&song, &[], 0).root, 0);
        // D minor (natural), its key range 50 … 62.
        song.key_root = 2;
        song.key_scale =
            scale_to_stored(SCALES.iter().position(|&s| s == "Minor Natural").unwrap());
        let file = &mut song.channels[1].rack.modules[0].file;
        (file.key_low, file.key_high) = (50, 62);
        assert_eq!(light(&song), [50, 52, 53, 55, 57, 58, 60, 62]);
        assert_eq!(roll.rows(&song, &[], 0).root, 2);
        // No instrument on the channel: no light rows.
        song.channels[1].rack.modules.clear();
        assert_eq!(light(&song), Vec::<usize>::new());
        // A plugin instrument (Surge XT, Lemon Trio) plays every key: the scale across 0–127.
        let id = song.channels[1]
            .rack
            .add(crate::song::plugin::PLUGIN_INSTRUMENT);
        song.channels[1].rack.get_mut(id).unwrap().file.key_high = 0;
        let d_minor = (0..128).filter(|k| [0, 2, 4, 5, 7, 9, 10].contains(&(k % 12)));
        assert_eq!(light(&song), d_minor.collect::<Vec<usize>>());
        song.channels[1].rack.modules.clear();
        // Lit: a selected note's row, and the keys sounding on the channel (the keyboard, typed, MIDI).
        let mut roll = roll;
        let events = [n(0, 128.0, 57)];
        roll.selected = vec![true];
        let lit = roll.rows(&song, &events, 1 << 40 | 1 << 127).lit;
        assert_eq!(
            (0..128).filter(|&k| lit[k]).collect::<Vec<usize>>(),
            [40, 57, 127]
        );
    }
}
