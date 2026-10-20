use eframe::egui::{Pos2, Rect, pos2};

use super::edit::*;
use super::*;
use crate::song::{Clip, Event, Lane, LaneKind, TimeSig};
use crate::ui::clips::TimeAxis;
use crate::ui::graph_lane::Prop;

/// A drum hit of `pad` at `pos` ticks, `ticks` long, velocity 100, pan 10.
fn drum(pad: u8, pos: u32, ticks: f64) -> Event {
    Event::Drum {
        pos,
        len_beats: ticks / BEAT,
        pad,
        velocity: 100,
        pan: 10,
        value: 7,
        repeat: 0,
    }
}

/// (pad, start, length) of every hit, in ticks.
fn hits(events: &[Event]) -> Vec<(u8, f64, f64)> {
    events.iter().filter_map(hit).collect()
}

fn clip(beats: f64) -> Clip {
    Lane::new(LaneKind::Drums).add_clip(0, beats).clone()
}

// The time axis.

#[test]
fn the_beat_is_u_d_over_4k_with_k_from_0_01_to_1_and_a_new_clip_at_4_u_per_beat() {
    let u = 32.0;
    // At d = 4: 1 U … 100 U per beat.
    assert_eq!(beat_w(u, 4, 1.0), 32.0);
    assert_eq!(beat_w(u, 4, 0.01), 3200.0);
    assert_eq!(beat_w(u, 4, 0.25), 128.0);
    // k is clamped to its range.
    assert_eq!(beat_w(u, 4, 2.0), 32.0);
    assert_eq!(beat_w(u, 4, 0.001), 3200.0);
    // A clip with no saved view: 4 U per beat, whatever d.
    for d in [2, 4, 8] {
        assert_eq!(beat_w(u, d, new_k(d)), 128.0, "d = {d}");
    }
    assert_eq!(new_k(4), 0.25);
}

#[test]
fn the_horizontal_scroll_range_is_twice_the_clip_length() {
    // A 4-beat clip at 128 points a beat: 8 beats of content, 5 of them in a 640-point grid.
    let max = |loop_beats: f64, grid_w: f32| {
        crate::ui::scroller::Axis {
            pos: 0.0,
            zoom: 1.0 / 128.0,
            range: [0.0, 2.0 * loop_beats],
            zoom_range: [1.0 / 128.0; 2],
            view: eframe::egui::Rangef::new(0.0, grid_w),
        }
        .max()
    };
    assert_eq!(max(4.0, 640.0), 3.0);
    assert_eq!(max(4.0, 2000.0), 0.0);
    assert_eq!(max(8.0, 640.0), 11.0);
}

#[test]
fn marks_are_4_px_bars_2_px_beats_1_px_finer_never_closer_than_0_6_u() {
    let u = 32.0;
    let ticks = |m: Vec<(f64, f32)>| {
        m.into_iter()
            .map(|(t, w)| (t as u32, w as u32))
            .collect::<Vec<_>>()
    };
    // 4 U a beat: quarter beats are 1 U apart, eighths 0.5 U (too close).
    let m = ticks(marks(0.0, 512.0, 128.0, u, TimeSig::default(), MARK_GAP));
    assert_eq!(m.len(), 17);
    assert_eq!(
        &m[..6],
        [(0, 4), (32, 1), (64, 1), (96, 1), (128, 2), (160, 1)]
    );
    assert_eq!(m[16], (512, 4));
    // 1 U a beat (k = 1): halves would be 0.5 U apart, so beats and bars only.
    assert_eq!(
        ticks(marks(0.0, 512.0, 32.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (128, 2), (256, 2), (384, 2), (512, 4)]
    );
    // 100 U a beat (k = 0.01): every tick (1/128 beat, 0.78 U).
    assert_eq!(
        ticks(marks(0.0, 4.0, 3200.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (1, 1), (2, 1), (3, 1), (4, 1)]
    );
    // Beats 0.5 U apart are not marked; bars always are.
    assert_eq!(
        ticks(marks(0.0, 1024.0, 16.0, u, TimeSig::default(), MARK_GAP)),
        [(0, 4), (512, 4), (1024, 4)]
    );
    // From the first mark at or after t0.
    assert_eq!(
        ticks(marks(100.0, 300.0, 128.0, u, TimeSig::default(), MARK_GAP)),
        [(128, 2), (160, 1), (192, 1), (224, 1), (256, 2), (288, 1)]
    );
}

#[test]
fn finer_marks_split_the_beat_by_the_subdivision_first() {
    // The timeline grid is 1/d beat (`TDIV` byte 0): the beat splits by d's factors first, then halves.
    let sig = |d| TimeSig {
        subdivision: d,
        beats_per_bar: 3,
    };
    let in_units = |m: Vec<(f64, f32)>, per_beat: f64| {
        (m.into_iter())
            .map(|(t, w)| ((t * per_beat / BEAT).round() as u32, w as u32))
            .collect::<Vec<_>>()
    };
    // d = 3, 2 U a beat: thirds are 0.67 U apart (sixths too close); a bar is 3 beats.
    let m = marks(0.0, 512.0, 64.0, 32.0, sig(3), MARK_GAP);
    assert!((m[1].0 - BEAT / 3.0).abs() < 1e-9);
    assert_eq!(
        in_units(m, 3.0),
        [
            (0, 4),
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 1),
            (5, 1),
            (6, 2),
            (7, 1),
            (8, 1),
            (9, 4),
            (10, 1),
            (11, 1),
            (12, 2)
        ]
    );
    // d = 6, 4 U a beat: sixths 0.67 U apart.
    assert_eq!(
        in_units(marks(0.0, 128.0, 128.0, 32.0, sig(6), MARK_GAP), 6.0),
        [(0, 4), (1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 2)]
    );
    // d = 6, 1.5 U a beat: sixths are too close, so halves (thirds would not fit either).
    assert_eq!(
        in_units(marks(0.0, 128.0, 48.0, 32.0, sig(6), MARK_GAP), 2.0),
        [(0, 4), (1, 1), (2, 2)]
    );
}

// Hits.

#[test]
fn a_hit_is_its_pad_start_and_length_in_ticks() {
    assert_eq!(hit(&drum(2, 64, 64.0)), Some((2, 64.0, 64.0)));
    let note = Event::Note {
        pos: 0,
        len_beats: 1.0,
        key: 60,
        velocity: 1,
        pan: 1,
        slide: false,
    };
    assert_eq!(hit(&note), None);
}

#[test]
fn the_minimum_length_is_an_eighth_of_the_snap_unit_or_1_128_beat_with_snap_off() {
    assert_eq!(min_len(Some(32.0)), 4.0);
    assert_eq!(min_len(Some(128.0)), 16.0);
    assert_eq!(min_len(None), 1.0);
}

#[test]
fn a_new_hit_starts_on_the_nearest_snap_unit_with_the_remembered_values() {
    let r = Remembered {
        len_beats: 0.5,
        velocity: 90,
        pan: 30,
    };
    let start = |tick, snap| hit(&new_hit(1, tick, snap, &r)).map(|h| h.1);
    // The nearest unit, not the one below; never before 0.
    assert_eq!(start(47.0, Some(32.0)), Some(32.0));
    assert_eq!(start(49.0, Some(32.0)), Some(64.0));
    assert_eq!(start(-10.0, Some(32.0)), Some(0.0));
    // Snap off: the tap's own tick.
    assert_eq!(start(47.4, None), Some(47.0));
    assert_eq!(
        new_hit(1, 47.0, Some(32.0), &r),
        Event::Drum {
            pos: 32,
            len_beats: 0.5,
            pad: 1,
            velocity: 90,
            pan: 30,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        }
    );
    // The remembered length never goes below the minimum; the defaults are 1/4 beat, 255 and 127.
    let short = Remembered {
        len_beats: 0.001,
        ..r
    };
    assert_eq!(hit(&new_hit(0, 0.0, Some(32.0), &short)).unwrap().2, 4.0);
    assert_eq!(hit(&new_hit(0, 0.0, None, &short)).unwrap().2, 1.0);
    let d = Remembered::default();
    assert_eq!((d.len_beats, d.velocity, d.pan), (0.25, 255, 127));
}

#[test]
fn insert_keeps_the_clip_sorted_and_the_new_hit_unselected() {
    let mut events = vec![drum(0, 0, 32.0), drum(1, 32, 32.0), drum(2, 64, 32.0)];
    let mut sel = vec![true, false, true];
    assert_eq!(insert(&mut events, &mut sel, drum(3, 32, 8.0)), 2);
    assert_eq!(hits(&events)[2], (3, 32.0, 8.0));
    assert_eq!(sel, [true, false, false, true]);
}

#[test]
fn a_move_snaps_the_anchor_start_keeps_the_group_after_0_and_its_rows_in_range() {
    let orig = [drum(0, 0, 32.0), drum(1, 64, 32.0), drum(2, 128, 32.0)];
    let sel = [true, true, false];
    let which = [true, true, false];
    let mv = |anchor, dt, dr, snap| moved(&orig, &sel, &which, anchor, dt, dr, 4, snap);
    // 40 ticks on: the anchor's start goes to the nearest 32; a row down. The third hit stays.
    let (events, s) = mv(0, 40.0, 1, Some(32.0));
    assert_eq!(
        hits(&events),
        [(1, 32.0, 32.0), (2, 96.0, 32.0), (2, 128.0, 32.0)]
    );
    assert_eq!(s, [true, true, false]);
    // Re-sorted, the selection with it.
    let (events, s) = mv(0, 150.0, 0, Some(32.0));
    assert_eq!(
        hits(&events),
        [(2, 128.0, 32.0), (0, 160.0, 32.0), (1, 224.0, 32.0)]
    );
    assert_eq!(s, [false, true, true]);
    // Not before the clip start: the group's first hit stops at 0.
    assert_eq!(
        hits(&mv(1, -100.0, 0, Some(32.0)).0)[..2],
        [(0, 0.0, 32.0), (1, 64.0, 32.0)]
    );
    // Rows clamp to the existing ones, the group's shape kept.
    assert_eq!(
        hits(&mv(0, 0.0, 5, Some(32.0)).0)[..2],
        [(2, 0.0, 32.0), (3, 64.0, 32.0)]
    );
    assert_eq!(
        hits(&mv(0, 0.0, -3, Some(32.0)).0)[..2],
        [(0, 0.0, 32.0), (1, 64.0, 32.0)]
    );
    // Snap off: whole ticks; overlaps are allowed.
    assert_eq!(
        hits(&mv(0, 40.4, 0, None).0)[..2],
        [(0, 40.0, 32.0), (1, 104.0, 32.0)]
    );
    let (events, _) = mv(0, 128.0, 2, Some(32.0));
    assert_eq!(hits(&events)[..2], [(2, 128.0, 32.0), (2, 128.0, 32.0)]);
}

#[test]
fn a_resize_snaps_the_anchor_end_and_keeps_the_minimum_length() {
    let orig = [drum(0, 0, 32.0), drum(1, 64, 64.0)];
    let len = |anchor, which: &[bool], dt, snap| -> Vec<f64> {
        hits(&resized(&orig, which, anchor, dt, snap))
            .iter()
            .map(|h| h.2)
            .collect()
    };
    // End 32 + 20 = 52 → 64.
    assert_eq!(len(0, &[true, false], 20.0, Some(32.0)), [64.0, 64.0]);
    // Both, by the anchor's change.
    assert_eq!(len(0, &[true, true], 32.0, Some(32.0)), [64.0, 96.0]);
    // Down to the minimum: 32/8, or a tick with snap off.
    assert_eq!(len(0, &[true, false], -40.0, Some(32.0)), [4.0, 64.0]);
    assert_eq!(len(0, &[true, false], -40.0, None), [1.0, 64.0]);
    assert_eq!(len(0, &[true, false], 3.4, None), [35.0, 64.0]);
}

#[test]
fn copy_delete_and_snap_act_on_the_selection() {
    let mut events = vec![drum(0, 0, 32.0), drum(1, 96, 32.0), drum(2, 40, 8.0)];
    events.sort_by_key(Event::pos);
    // [0: pad 0 at 0, 1: pad 2 at 40, 2: pad 1 at 96]
    let mut sel = vec![true, false, true];
    // Span 0..128 → copies 128 on, which become the selection.
    copy_selected(&mut events, &mut sel, 32.0);
    assert_eq!(
        hits(&events),
        [
            (0, 0.0, 32.0),
            (2, 40.0, 8.0),
            (1, 96.0, 32.0),
            (0, 128.0, 32.0),
            (1, 224.0, 32.0)
        ]
    );
    assert_eq!(sel, [false, false, false, true, true]);
    // A lone zero-length hit copies one unit on.
    let mut one = vec![drum(0, 64, 0.0)];
    let mut s = vec![true];
    copy_selected(&mut one, &mut s, 32.0);
    assert_eq!(hits(&one), [(0, 64.0, 0.0), (0, 96.0, 0.0)]);
    // Snap: the nearest grid line.
    let mut sel = vec![false, true, false, false, false];
    quantize_selected(&mut events, &mut sel, 32.0);
    assert_eq!(hits(&events)[1], (2, 32.0, 8.0));
    let mut sel = vec![false, true, false, true, false];
    crate::ui::piano_roll::notes::remove_selected(&mut events, &mut sel);
    assert_eq!(
        hits(&events),
        [(0, 0.0, 32.0), (1, 96.0, 32.0), (1, 224.0, 32.0)]
    );
    assert_eq!(sel, [false; 3]);
}

#[test]
fn paste_puts_the_earliest_hit_on_the_tick_and_the_others_at_their_offsets_selected() {
    // As the automation editor's: pads and lengths as copied.
    let mut events = vec![drum(0, 0, 32.0)];
    let mut sel = vec![true];
    paste(
        &mut events,
        &mut sel,
        &[drum(1, 96, 16.0), drum(2, 40, 8.0)],
        200.0,
        None,
    );
    assert_eq!(
        hits(&events),
        [(0, 0.0, 32.0), (2, 200.0, 8.0), (1, 256.0, 16.0)]
    );
    assert_eq!(sel, [false, true, true]);
}

/// Row `pad`'s hits as (position in ticks, length in beats, velocity), in clip order.
fn fill_row(c: &Clip, pad: u8) -> Vec<(u32, f64, u8)> {
    (c.events.iter())
        .filter_map(|e| match *e {
            Event::Drum {
                pos,
                len_beats,
                velocity,
                pad: p,
                ..
            } if p == pad => Some((pos, len_beats, velocity)),
            _ => None,
        })
        .collect()
}

#[test]
fn fill_places_hits_every_n_steps_n_quarter_beats_long_at_velocity_255() {
    for every in [1usize, 2, 4] {
        let mut c = clip(4.0);
        c.events = vec![drum(0, 32, 32.0), drum(1, 33, 32.0), drum(1, 600, 32.0)];
        fill(&mut c, 1, every, TimeSig::default());
        let want: Vec<_> = (0..16)
            .step_by(every)
            .map(|s| (s as u32 * 32, every as f64 / 4.0, 255))
            .collect();
        assert_eq!(fill_row(&c, 1), want, "every {every}");
        // Other rows stay; every hit of the row goes first, past the loop too.
        assert!(hits(&c.events).contains(&(0, 32.0, 32.0)));
        assert!(!hits(&c.events).contains(&(1, 600.0, 32.0)));
    }
}

/// At d = TDIV byte 0 = 3 the hits are N/3 beat apart over llround(loop·3) subdivisions, and still N/4 beat long (not
/// N/3), so they overlap. A third of a beat falls between ticks: it rounds to the nearer.
#[test]
fn fill_at_3_steps_a_beat_spaces_hits_n_thirds_apart_n_quarter_beats_long() {
    let sig = |d| TimeSig {
        subdivision: d,
        beats_per_bar: 4,
    };
    let thirds = [0, 43, 85, 128, 171, 213, 256, 299, 341, 384, 427, 469];
    for every in [1usize, 2, 4] {
        let mut c = clip(4.0);
        c.events = vec![drum(1, 600, 32.0)];
        fill(&mut c, 1, every, sig(3));
        let want: Vec<_> = (thirds.iter().step_by(every))
            .map(|&pos| (pos, every as f64 / 4.0, 255))
            .collect();
        assert_eq!(fill_row(&c, 1), want, "every {every}");
    }
    // llround: 2.1 beats at d = 3 are 6.3 subdivisions, rounded to 6 (ceil would make 7).
    let mut c = clip(2.1);
    fill(&mut c, 0, 1, sig(3));
    assert_eq!(fill_row(&c, 0).len(), 6);
    // A d of 0 counts as 1 (as on the time axis): a hit a beat.
    let mut c = clip(4.0);
    fill(&mut c, 0, 1, sig(0));
    assert_eq!(
        fill_row(&c, 0),
        [
            (0, 0.25, 255),
            (128, 0.25, 255),
            (256, 0.25, 255),
            (384, 0.25, 255)
        ]
    );
}

/// The free grid fills the whole loop: the step grid's 512-step cap is not its. At d = 8 an 80-beat loop is 640
/// subdivisions, the last at 639/8 beat.
#[test]
fn fill_covers_a_free_grid_loop_past_the_step_grids_512_steps() {
    let sig = TimeSig {
        subdivision: 8,
        beats_per_bar: 4,
    };
    let mut c = clip(80.0);
    fill(&mut c, 0, 1, sig);
    let row = fill_row(&c, 0);
    assert_eq!(row.len(), 640);
    assert_eq!(row.last(), Some(&(10_224, 0.25, 255)));
}

#[test]
fn the_loop_fits_the_hits_ends_in_power_of_two_bars() {
    let mut c = clip(4.0);
    // A hit's end counts: 511 + 64 needs a second bar.
    c.events = vec![drum(0, 511, 64.0)];
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
    c.events = vec![drum(0, 511, 0.0), drum(0, 512, 0.0)];
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
    // Bars double (16, not 12). A new clip (visible length 0) shows its loop.
    c.events.push(drum(0, 1100, 0.0));
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (16.0, 0.0));
    assert_eq!(c.visible_ticks(), 16.0 * 128.0);
    // It shrinks back to one bar (the generic rule), the length with it.
    c.events.clear();
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 0.0));
    assert_eq!(c.visible_ticks(), 4.0 * 128.0);
    // A resized clip's length stays, one equal to the loop included.
    c.visible_len_beats = 4.0;
    c.events.push(drum(0, 1100, 0.0));
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (16.0, 4.0));
    c.events.clear();
    c.fit_loop(4);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (4.0, 4.0));
}

#[test]
fn the_graph_paint_sets_the_rows_eligible_hits_whose_extent_holds_the_tick() {
    let orig = vec![drum(0, 0, 32.0), drum(0, 64, 0.0), drum(1, 0, 32.0)];
    let vel = |e: &[Event]| {
        e.iter()
            .map(|e| Prop::Velocity.get(e).unwrap())
            .collect::<Vec<_>>()
    };
    let mut events = orig.clone();
    assert!(paint(&mut events, &[false; 3], 0, Prop::Velocity, 16.0, 50));
    assert_eq!(vel(&events), [50, 100, 100]);
    // The end is in the extent; the same value again changes nothing.
    assert!(!paint(
        &mut events,
        &[false; 3],
        0,
        Prop::Velocity,
        32.0,
        50
    ));
    // A zero-length hit at exactly its start.
    assert!(paint(&mut events, &[false; 3], 0, Prop::Velocity, 64.0, 9));
    assert!(!paint(&mut events, &[false; 3], 0, Prop::Velocity, 64.5, 1));
    assert_eq!(vel(&events), [50, 9, 100]);
    // While any is selected, only the selected.
    let mut events = orig.clone();
    assert!(!paint(
        &mut events,
        &[false, true, false],
        0,
        Prop::Pan,
        16.0,
        0
    ));
    assert!(paint(
        &mut events,
        &[false, true, false],
        0,
        Prop::Pan,
        64.0,
        0
    ));
}

// Geometry.

/// Tick 0 at x 100, 128 points a beat; rows 64 points from y 48; four rows.
const PLACE: Place = Place {
    axis: TimeAxis {
        origin: 100.0,
        beat_w: 128.0,
    },
    row_top: 48.0,
    pitch: 64.0,
    rows: 4,
    u: 32.0,
};

#[test]
fn a_hits_extent_is_its_start_to_its_end_on_its_row() {
    assert_eq!(
        PLACE.rect(&drum(1, 64, 32.0)),
        Some(Rect::from_min_max(pos2(164.0, 112.0), pos2(196.0, 176.0)))
    );
    assert_eq!(PLACE.row(47.0), None);
    assert_eq!(PLACE.row(48.0), Some(0));
    assert_eq!(PLACE.row(303.0), Some(3));
    assert_eq!(PLACE.row(304.0), None);
}

#[test]
fn hit_testing_uses_the_extent_with_no_minimum_width_topmost_first() {
    let events = [drum(1, 64, 32.0), drum(1, 128, 0.0), drum(1, 80, 32.0)];
    let at = |x: f32| PLACE.hit_at(&events, Pos2::new(x, 140.0));
    assert_eq!(at(165.0), Some(0));
    // Overlapping: the later one, drawn on top.
    assert_eq!(at(190.0), Some(2));
    assert_eq!(at(212.0), Some(2));
    assert_eq!(at(212.5), None);
    // A zero-length hit only at its start.
    assert_eq!(at(228.0), Some(1));
    assert_eq!(at(228.5), None);
    // Another row.
    assert_eq!(PLACE.hit_at(&events, pos2(165.0, 100.0)), None);
}

#[test]
fn the_marquee_selects_hits_whose_extent_overlaps_it() {
    let events = [drum(1, 64, 32.0), drum(1, 128, 0.0), drum(3, 0, 32.0)];
    let marquee = |a: Pos2, b: Pos2| PLACE.in_box(&events, Rect::from_two_pos(a, b));
    assert_eq!(
        marquee(pos2(150.0, 100.0), pos2(170.0, 120.0)),
        [true, false, false]
    );
    assert_eq!(
        marquee(pos2(220.0, 130.0), pos2(230.0, 130.0)),
        [false, true, false]
    );
    assert_eq!(marquee(pos2(0.0, 0.0), pos2(400.0, 400.0)), [true; 3]);
    assert_eq!(marquee(pos2(197.0, 130.0), pos2(227.0, 300.0)), [false; 3]);
}

#[test]
fn a_handles_touch_box_is_the_piano_rolls_on_the_end_capsule() {
    // 0.6 U wide and 1.41 U tall, centred 0.36 U past the end (on the capsule) on the row's centre line.
    let h = PLACE.handle(&drum(1, 64, 32.0)).unwrap();
    assert!((h.center() - pos2(196.0 + 11.52, 144.0)).length() < 1e-4);
    assert!((h.size() - vec2(19.2, 45.12)).length() < 1e-4);
}

#[test]
fn the_pop_in_scales_from_nothing_past_full_size_and_settles() {
    // a ← 0.6·a per 60 Hz frame; the scale 1.0515·sin(1.885·(1 − a)).
    assert_eq!(pop_scale(1.0), 0.0);
    let frame = 1.0 / 60.0;
    let scale = |frames: f64| pop_scale(pop_in(POP_RATE, frames * frame));
    assert!((scale(1.0) - 0.7199).abs() < 1e-3, "{}", scale(1.0));
    assert!(scale(3.0) > 1.04, "it overshoots: {}", scale(3.0));
    // 0.6^9 is just over 0.01, 0.6^10 under: over, full size.
    assert!(pop_in(POP_RATE, 9.0 * frame) > 0.0);
    assert_eq!(pop_in(POP_RATE, 10.0 * frame), 0.0);
    assert_eq!(scale(10.0), 1.0);
}

// Offscreen egui: the view in a real context, driven by synthetic input. Each action is applied to the song.

use eframe::egui::{self, Color32, Context, Modifiers, PointerButton, RawInput, Vec2, vec2};

use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::song::{Channel, ChannelKind, DrumGrid, Song};
use crate::theme::{channel_color, theme_by_id};
use crate::ui::UiCtx;
use crate::ui::step_seq::{DrumPads, StepAction, hit_clipboard, hit_color, hit_tints};

/// A fake kit of four pads.
struct Kit;

impl DrumPads for Kit {
    fn count(&self) -> usize {
        4
    }

    fn names(&self, pad: usize) -> (&str, &str) {
        [
            ("Kick", "808 Kick"),
            ("Snare", "808 Snare"),
            ("Hat", "808 Hat"),
            ("Clap", "808 Clap"),
        ][pad]
    }

    fn midi_note(&self, _: usize) -> Option<u8> {
        None
    }

    fn custom_name(&self, _: usize) -> &str {
        ""
    }
}

const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1170.0, 540.0),
};
/// The channel under test, and the left of its grid (the 3 U header column and the t gap, ceil(U/16) px).
const CH: usize = 1;
const GRID_X: f32 = 98.0;
/// The rows' top at U = 32 (ppp 1): the ruler's height; row 0 starts on the nearest pixel, [`Y0`].
const TOP: f32 = 52.0;
const Y0: f32 = 52.0;

/// Row `pad`'s centre line at `tick`, at the default view: 4 U (128 points) a beat is a point a tick from the grid's
/// left; rows 2 U (64 points) from below the ruler.
fn at(pad: usize, tick: f32) -> Pos2 {
    pos2(GRID_X + tick, TOP + 64.0 * pad as f32 + 32.0)
}

/// [`notes_lane`] with no hits in the one-bar clip at 0 a playlist `+Clip` makes: the free grid makes no clip itself.
fn empty_clip() -> Channel {
    let mut channel = notes_lane(&[]);
    channel.lanes[0].add_clip(0, 4.0);
    channel
}

/// A drums channel whose lane 0 is a notes lane holding `hits` in a one-bar clip (no clip without hits).
fn notes_lane(hits: &[Event]) -> Channel {
    let mut song = Song::default();
    song.add_channel(ChannelKind::Drums);
    let mut channel = song.channels.remove(1);
    channel.lanes[0].grid = DrumGrid::Notes;
    if !hits.is_empty() {
        channel.lanes[0].add_clip(0, 4.0).events = hits.to_vec();
    }
    channel
}

struct Rig {
    ctx: Context,
    view: DrumGridView,
    song: Song,
    /// The clip the view edits, by id: the lane's first, as the app opens it; none without one.
    clip: Option<u32>,
    remembered: Remembered,
    ppp: f32,
    time: f64,
    actions: Vec<StepAction>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl Rig {
    fn new(channel: Channel) -> Self {
        Self::at_ppp(channel, 1.0)
    }

    fn at_ppp(channel: Channel, ppp: f32) -> Self {
        let clip = channel
            .lanes
            .first()
            .and_then(|l| l.clips.first())
            .map(|c| c.id);
        let mut song = Song::default();
        song.channels.push(channel);
        let mut rig = Self {
            ctx: Context::default(),
            view: DrumGridView::default(),
            song,
            clip,
            remembered: Remembered::default(),
            ppp,
            time: 0.0,
            actions: Vec::new(),
            shapes: Vec::new(),
        };
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        rig.actions.clear();
        rig
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.frame_after(0.1, events);
    }

    fn frame_after(&mut self, dt: f64, events: Vec<egui::Event>) {
        self.time += dt;
        let mut raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(raw.viewport_id).or_default();
        viewport.native_pixels_per_point = Some(self.ppp);
        let (theme, m) = (
            theme_by_id(0),
            Metrics::from_pixels_per_point(self.ppp, DEFAULT_UI_SCALE),
        );
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let (view, song, clip, remembered) =
            (&mut self.view, &self.song, self.clip, &mut self.remembered);
        let mut action = None;
        let output = self.ctx.run(raw, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = view.show(ui, &cx, song, CH, (0, clip), &Kit, remembered);
                    view.graph_knob(ui, &cx, SCREEN, egui::Color32::WHITE);
                });
        });
        self.shapes = output.shapes;
        if let Some(action) = action {
            self.view.apply(&mut self.song, CH, (0, self.clip), &action);
            self.actions.push(action);
        }
    }

    fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn click_with(&mut self, pos: Pos2, button: PointerButton) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, button, true);
        self.button(pos, button, false);
        // Lets a popup or the floating row lay itself out.
        self.frame(vec![]);
    }

    fn click(&mut self, pos: Pos2) {
        self.click_with(pos, PointerButton::Primary);
    }

    /// Presses at `from`, moves through `offsets` (from the press), one frame each, and releases.
    fn drag(&mut self, from: Pos2, offsets: &[Vec2]) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.button(from, PointerButton::Primary, true);
        for &d in offsets {
            self.frame(vec![egui::Event::PointerMoved(from + d)]);
        }
        let end = from + offsets.last().copied().unwrap_or_default();
        self.button(end, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    /// The primary button pressed at `pos` and held there for a second: a long press.
    fn hold(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.button(pos, PointerButton::Primary, true);
        self.frame_after(1.0, vec![]);
    }

    fn release(&mut self, pos: Pos2) {
        self.button(pos, PointerButton::Primary, false);
        self.frame(vec![]);
    }

    fn events(&self) -> &[Event] {
        let lane = &self.song.channels[CH].lanes[0];
        lane.clips.first().map_or(&[], |c| &c.events)
    }

    fn hits(&self) -> Vec<(u8, f64, f64)> {
        hits(self.events())
    }

    fn selected(&self) -> Vec<bool> {
        self.view.sel.clone()
    }

    fn text_at(&self, label: &str) -> Option<Pos2> {
        self.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(t.visual_bounding_rect().center())
            }
            _ => None,
        })
    }

    fn rects(&self, color: Color32) -> Vec<Rect> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == color => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// A resize handle is drawn at `center`: a `text` disc 0.6 U (19.2 points) across.
    /// A handle's capsule body (a rounded `text` rect) just past the hit end `end`, centred on its row.
    fn handle_at(&self, end: Pos2) -> bool {
        let text = theme_by_id(0).colors.text.to_color32();
        self.shapes.iter().any(|s| {
            matches!(&s.shape, egui::Shape::Rect(r)
                if r.fill == text && r.corner_radius != egui::CornerRadius::ZERO
                    && (end.x..end.x + 8.0).contains(&r.rect.left()) && r.rect.center().y == end.y)
        })
    }

    /// The hit colour on page `prop` of a hit at `value`.
    fn hit_color(&self, prop: Prop, value: u16, selected: bool) -> Color32 {
        let theme = theme_by_id(0);
        let color = channel_color(self.song.channels[CH].hue, &theme);
        hit_color(
            &hit_tints(&theme.colors, color, prop),
            prop,
            value,
            selected,
        )
    }

    /// The `Hits` edits, as their final flags.
    fn finals(&self) -> Vec<bool> {
        (self.actions.iter())
            .filter_map(|a| match a {
                StepAction::Hits { final_edit, .. } => Some(*final_edit),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn the_view_edits_its_clip_by_id_and_a_gone_id_edits_nothing() {
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].add_clip(512, 4.0);
    let mut rig = Rig::new(channel);
    // Clip 2, the lane's second: the tap lands there.
    rig.clip = Some(2);
    rig.frame(vec![]);
    rig.click(at(1, 47.0));
    let counts = |rig: &Rig| -> Vec<usize> {
        (rig.song.channels[CH].lanes[0].clips.iter())
            .map(|c| c.events.len())
            .collect()
    };
    assert_eq!(counts(&rig), [1, 1]);
    // An id that is gone: no other clip takes its place.
    rig.clip = Some(9);
    rig.frame(vec![]);
    rig.click(at(2, 47.0));
    assert_eq!(counts(&rig), [1, 1]);
}

#[test]
fn a_lane_with_no_clip_takes_no_hit_and_makes_no_clip() {
    // The playlist's `+Clip` makes clips; the grid edits the one it is on.
    let mut rig = Rig::new(notes_lane(&[]));
    rig.click(at(1, 47.0));
    assert!(rig.song.channels[CH].lanes[0].clips.is_empty());
}

#[test]
fn a_tap_on_an_empty_cell_adds_a_hit_with_the_remembered_values_in_one_final_edit() {
    let mut rig = Rig::new(empty_clip());
    rig.click(at(1, 47.0));
    assert_eq!(rig.finals(), [true]);
    // A tap's new hit re-syncs nothing.
    let quiet = matches!(rig.actions[..], [StepAction::Hits { quiet: true, .. }]);
    assert!(quiet, "{:?}", rig.actions);
    assert_eq!(
        rig.events(),
        [Event::Drum {
            pos: 32,
            len_beats: 0.25,
            pad: 1,
            velocity: 255,
            pan: 127,
            value: Event::DEFAULT_PITCH,
            repeat: 0,
        }]
    );
    // The remembered values; the nearest snap unit.
    rig.remembered = Remembered {
        len_beats: 0.5,
        velocity: 90,
        pan: 30,
    };
    rig.click(at(2, 113.0));
    assert_eq!(rig.hits()[1], (2, 128.0, 64.0));
    assert!(matches!(
        rig.events()[1],
        Event::Drum {
            velocity: 90,
            pan: 30,
            ..
        }
    ));
    // Snap off: the tap's own tick. The new hits are not selected.
    rig.view.snap.on = false;
    rig.click(at(3, 47.0));
    assert_eq!(rig.hits()[1], (3, 47.0, 64.0));
    assert_eq!(rig.finals(), [true; 3]);
    assert_eq!(rig.selected(), [false; 3]);
}

#[test]
fn a_tap_on_a_hit_selects_it_with_a_handle_and_copy_delete_snap_and_a_second_tap_deselects_it() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 64.0), drum(1, 64, 32.0)]));
    rig.click(at(1, 80.0));
    assert_eq!(rig.selected(), [false, true]);
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    for label in OPS {
        let p = rig.text_at(label).unwrap_or_else(|| panic!("{label}"));
        // Above the one-row selection.
        assert!(p.y < Y0 + 64.0, "{label} at {p:?}");
    }
    // The handle: the end capsule past the hit's end.
    assert!(rig.handle_at(pos2(GRID_X + 96.0, at(1, 0.0).y)));
    // Remembered: its length, velocity and pan.
    assert_eq!(
        rig.remembered,
        Remembered {
            len_beats: 0.25,
            velocity: 100,
            pan: 10
        }
    );
    // Another tap on it deselects it: no handle, no row. Nothing was edited.
    rig.click(at(1, 80.0));
    assert_eq!(rig.selected(), [false, false]);
    assert_eq!(rig.text_at("Copy"), None);
    assert!(!rig.handle_at(pos2(192.0, at(1, 0.0).y)));
    // While one shows, a tap on another hit only deselects (below the floating row over the Kick's row).
    rig.click(at(1, 80.0));
    let kick = at(0, 10.0) + vec2(0.0, 29.0);
    rig.click(kick);
    assert_eq!(rig.selected(), [false, false]);
    rig.click(kick);
    assert_eq!(rig.selected(), [true, false]);
    assert!(rig.actions.is_empty());
}

#[test]
fn copy_fills_the_hit_clipboard_and_a_long_press_on_empty_grid_pastes_it_at_the_snapped_beat() {
    // As the automation editor's `Copy`, `Paste` button and paste.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 64.0), drum(1, 64, 32.0)]));
    let empty = at(2, 384.0);
    rig.hold(empty);
    rig.release(empty);
    assert_eq!(rig.text_at("Paste"), None, "nothing to paste");
    rig.click(at(1, 80.0));
    rig.click(rig.text_at("Copy").unwrap());
    assert_eq!(hit_clipboard(&rig.ctx), [drum(1, 64, 32.0)]);
    rig.hold(empty);
    assert!(
        rig.text_at("Paste").is_some(),
        "Paste appears before lifting"
    );
    rig.release(empty);
    let paste = rig.text_at("Paste").expect("the Paste button");
    assert!(paste.y < empty.y, "over the finger's row");
    rig.click(paste);
    assert_eq!(rig.finals().last(), Some(&true));
    // Beat 3, the held one's: pasted, and the selection.
    let pasted = (rig.events().iter())
        .position(|e| e.pos() == 384)
        .expect("pasted on beat 3");
    assert_eq!(hits(&rig.events()[pasted..=pasted]), [(1, 384.0, 32.0)]);
    let mut want = vec![false; rig.events().len()];
    want[pasted] = true;
    assert_eq!(rig.selected(), want);
    assert_eq!(rig.text_at("Paste"), None);
    // A right click on empty grid shows it too; a press anywhere else hides it.
    rig.click_with(at(3, 700.0), PointerButton::Secondary);
    assert!(rig.text_at("Paste").is_some());
    rig.click(at(3, 900.0));
    assert_eq!(rig.text_at("Paste"), None);
}

#[test]
fn while_a_selection_shows_a_tap_on_empty_space_only_clears_it() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.click(at(0, 16.0));
    assert_eq!(rig.selected(), [true]);
    rig.click(at(2, 200.0));
    assert_eq!(rig.selected(), [false]);
    assert_eq!(rig.hits(), [(0, 0.0, 32.0)]);
    assert!(rig.actions.is_empty());
}

#[test]
fn a_drag_on_a_hit_moves_it_at_once_snapped_and_by_whole_rows_in_one_undo_step() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0), drum(3, 256, 32.0)]));
    // No long press: the press, then the moves.
    rig.drag(
        at(0, 16.0),
        &[vec2(10.0, 16.0), vec2(35.0, 32.0), vec2(70.0, 64.0)],
    );
    // 70 ticks on → 64 (the nearest step), a row down: the Snare.
    assert_eq!(rig.hits(), [(1, 64.0, 32.0), (3, 256.0, 32.0)]);
    // Ongoing edits, then the release's final one: one undo step.
    let finals = rig.finals();
    assert!(
        finals.len() >= 2 && finals[..finals.len() - 1].iter().all(|f| !f),
        "{finals:?}"
    );
    assert_eq!(finals.last(), Some(&true));
    // Touched, it became the selection, and is remembered.
    assert_eq!(rig.selected(), [true, false]);
    assert_eq!(rig.remembered.velocity, 100);
    // Rows clamp to the existing ones; not before the clip start.
    rig.drag(at(1, 80.0), &[vec2(-30.0, 100.0), vec2(-200.0, 400.0)]);
    assert_eq!(rig.hits(), [(3, 0.0, 32.0), (3, 256.0, 32.0)]);
    // A release in the frame of the last move: that move is the final edit.
    rig.actions.clear();
    let from = at(3, 16.0);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.button(from, PointerButton::Primary, true);
    rig.frame(vec![egui::Event::PointerMoved(from + vec2(20.0, 0.0))]);
    let to = from + vec2(64.0, 0.0);
    let up = egui::Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    };
    rig.frame(vec![egui::Event::PointerMoved(to), up]);
    assert_eq!(rig.hits(), [(3, 64.0, 32.0), (3, 256.0, 32.0)]);
    assert_eq!(rig.finals().last(), Some(&true));
}

#[test]
fn the_resize_handle_drags_the_end_to_the_nearest_snap_unit_down_to_the_minimum() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.click(at(0, 16.0));
    // The handle, 0.3 U past the end: 50 points on, the end 82 → 96.
    let handle = at(0, 32.0 + 9.6);
    rig.drag(handle, &[vec2(10.0, 0.0), vec2(50.0, 0.0)]);
    assert_eq!(rig.hits(), [(0, 0.0, 96.0)]);
    assert_eq!(rig.finals().last(), Some(&true));
    assert_eq!(rig.finals().iter().filter(|f| **f).count(), 1);
    assert_eq!(rig.selected(), [true]);
    // Back past the start: the minimum, 1/8 of the 32-tick step.
    rig.drag(at(0, 96.0 + 9.6), &[vec2(-100.0, 0.0), vec2(-200.0, 0.0)]);
    assert_eq!(rig.hits(), [(0, 0.0, 4.0)]);
}

#[test]
fn a_new_hit_takes_the_length_a_lone_hit_was_resized_to() {
    // As in the piano roll (a new note is as long as the last one pressed, resized ones included), in the drum
    // editor's terms: a resize of the lone selected hit is remembered at its new length.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.click(at(0, 16.0));
    assert_eq!(rig.remembered.len_beats, 0.25);
    // The handle: the end 32 → 96.
    rig.drag(at(0, 32.0 + 9.6), &[vec2(10.0, 0.0), vec2(50.0, 0.0)]);
    assert_eq!(rig.hits(), [(0, 0.0, 96.0)]);
    assert_eq!(rig.remembered.len_beats, 0.75);
    // A tap on empty space clears the selection; the next adds a hit of the resized length.
    rig.click(at(2, 200.0));
    rig.click(at(2, 200.0));
    assert_eq!(rig.hits()[1], (2, 192.0, 96.0));
}

#[test]
fn a_stationary_hold_on_empty_grid_has_no_marquee_until_dragged() {
    // The marquee, the mouse held 0.35 s: no egui drag start needed.
    let hits = [drum(0, 0, 32.0), drum(1, 64, 32.0), drum(2, 256, 32.0)];
    let mut rig = Rig::new(notes_lane(&hits));
    let from = at(1, 40.0);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.button(from, PointerButton::Primary, true);
    rig.frame_after(0.35, vec![]);
    let squares: Vec<Rect> = (rig.shapes.iter())
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if (r.rect.center() - from).length() < 0.5 => Some(r.rect),
            _ => None,
        })
        .collect();
    assert!(
        !squares.iter().any(|r| (r.width() - 96.0).abs() < 0.5),
        "{squares:?}"
    );
    assert_eq!(rig.selected(), [false; 3]);
    let to = at(2, 300.0);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    assert_eq!(rig.selected(), [false, true, true]);
    rig.release(to);
    assert_eq!(rig.selected(), [false, true, true]);
    assert!(rig.actions.is_empty());
}

#[test]
fn a_long_press_then_a_drag_is_a_marquee_selecting_hits_by_extent() {
    let hits = [
        drum(0, 0, 32.0),
        drum(1, 64, 0.0),
        drum(2, 256, 32.0),
        drum(3, 300, 32.0),
    ];
    let mut rig = Rig::new(notes_lane(&hits));
    let (from, to) = (at(1, 40.0), at(2, 70.0) + vec2(0.0, -20.0));
    rig.hold(from);
    rig.frame(vec![egui::Event::PointerMoved(from + (to - from) / 2.0)]);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    // The row waits for the marquee.
    assert_eq!(rig.text_at("Copy"), None);
    rig.release(to);
    // The zero-length hit at 64 lies in it; then its handle and the row show.
    assert_eq!(rig.selected(), [false, true, false, false]);
    assert!(rig.text_at("Copy").is_some());
    assert!(rig.handle_at(pos2(GRID_X + 64.0, at(1, 0.0).y)));
    assert!(rig.actions.is_empty());
    assert_eq!(rig.view.views.values().next().map(|v| v.x), Some(0.0));
    // A long press on empty space, lifted where it was, selects nothing and adds nothing.
    let at = at(3, 600.0);
    rig.hold(at);
    rig.release(at);
    assert_eq!(rig.selected(), [false; 4]);
    assert!(rig.actions.is_empty());
}

#[test]
fn the_floating_snap_copy_and_delete_are_each_one_final_edit() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 40, 32.0)]));
    rig.click(at(0, 50.0));
    let pick = |rig: &mut Rig, label: &str| {
        let p = rig.text_at(label).expect(label);
        rig.click(p);
    };
    // Snap: the nearest step.
    pick(&mut rig, "Snap");
    assert_eq!(rig.hits(), [(0, 32.0, 32.0)]);
    // Copy (the row stays over the selection): right after it, the copy alone selected and remembered.
    rig.remembered = Remembered::default();
    pick(&mut rig, "Copy");
    assert_eq!(rig.hits(), [(0, 32.0, 32.0), (0, 64.0, 32.0)]);
    assert_eq!(rig.selected(), [false, true]);
    assert_eq!(rig.remembered.velocity, 100);
    // Delete: the selected copy goes.
    pick(&mut rig, "Delete");
    assert_eq!(rig.hits(), [(0, 32.0, 32.0)]);
    assert_eq!(rig.finals(), [true; 3]);
}

#[test]
fn the_row_menu_hides_rotate_and_its_fill_makes_n_quarter_beat_hits() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    // The Snare's header, long-pressed.
    rig.click_with(pos2(48.0, 144.0), PointerButton::Secondary);
    assert!(rig.text_at("Fill each 2 step").is_some());
    assert_eq!(rig.text_at("Rotate right"), None);
    assert_eq!(rig.text_at("Rotate left"), None);
    let p = rig.text_at("Fill each 2 step").unwrap();
    rig.click(p);
    assert_eq!(rig.actions, [StepAction::Fill { pad: 1, every: 2 }]);
    let row: Vec<_> = rig.hits().into_iter().filter(|h| h.0 == 1).collect();
    let want: Vec<_> = (0..8).map(|i| (1, f64::from(i) * 64.0, 64.0)).collect();
    assert_eq!(row, want);
    assert!(rig.events().iter().all(|e| matches!(
        e,
        Event::Drum { pad: 0, .. } | Event::Drum { velocity: 255, .. }
    )));
}

#[test]
fn the_snap_button_has_the_ruler_cell_in_place_of_swing() {
    let mut rig = Rig::new(empty_clip());
    assert_eq!(rig.text_at("SWING"), None);
    // The shared Snap button in the 1.5 U × (ruler h − 1 px) cell, on: a tap toggles snap.
    assert!(rig.view.snap.on);
    rig.click(pos2(24.0, 24.0));
    assert!(!rig.view.snap.on);
    rig.click(pos2(24.0, 24.0));
    assert!(rig.view.snap.on);
    // A long press opens the snap menu; `Beat` snaps to beats.
    rig.hold(pos2(24.0, 24.0));
    rig.release(pos2(24.0, 24.0));
    assert!(rig.text_at("1/6 Step").is_some());
    let beat = rig.text_at("Beat").unwrap();
    rig.click(beat);
    assert_eq!(
        rig.view.snap,
        Snap {
            on: true,
            mode: SnapMode::Beat
        }
    );
    rig.click(at(0, 100.0));
    assert_eq!(rig.hits(), [(0, 128.0, 32.0)]);
}

#[test]
fn hits_are_drawn_over_their_length_2_px_into_the_row_on_a_1_px_background_frame() {
    for (ppp, px) in [(1.0, 1.0), (2.0, 0.5)] {
        let rig = Rig::at_ppp(notes_lane(&[drum(1, 64, 32.0), drum(2, 128, 0.0)]), ppp);
        let fill = rig.hit_color(Prop::Velocity, 100, false);
        let bg = theme_by_id(0).colors.background.to_color32();
        // Row 1 is Y0 + 64 … Y0 + 128 (the ruler cut to whole pixels: 52.5 points at ppp 2): the hit is 2 px in from
        // each.
        let (x, y) = (|tick: f32| GRID_X + tick, |y: f32| y + 2.0 * px);
        let y0 = (1.65 * 32.0 * ppp).trunc() / ppp;
        let row = |i: f32| y0 + 64.0 * i;
        let hit = Rect::from_min_max(
            pos2(x(64.0), y(row(1.0))),
            pos2(x(96.0), row(2.0) - 2.0 * px),
        );
        let zero = Rect::from_min_max(
            pos2(x(128.0), y(row(2.0))),
            pos2(x(128.0), row(3.0) - 2.0 * px),
        );
        assert_eq!(rig.rects(fill), [hit, zero], "ppp {ppp}");
        let frames = rig.rects(bg);
        assert!(frames.contains(&hit.expand(px)), "ppp {ppp}");
        assert!(frames.contains(&zero.expand(px)), "ppp {ppp}");
    }
}

#[test]
fn rows_are_control_fill_bands_under_background_marks() {
    let rig = Rig::new(notes_lane(&[]));
    let fill = theme_by_id(0).colors.control_fill.to_color32();
    let bg = theme_by_id(0).colors.background.to_color32();
    // Row 0, Y0 … Y0 + 64, inset 1 px; from the grid's left (beat 0) to its right.
    let bands = rig.rects(fill);
    assert!(
        bands.contains(&Rect::from_min_max(
            pos2(GRID_X, Y0 + 1.0),
            pos2(1170.0, Y0 + 63.0)
        )),
        "{bands:?}"
    );
    // At 4 U a beat: a 4 px bar line at tick 0, 2 px at the beat, 1 px at the step, over the four rows.
    let lines = rig.rects(bg);
    let line = |x0: f32, w: f32| Rect::from_min_max(pos2(x0, TOP), pos2(x0 + w, TOP + 256.0));
    for l in [
        line(GRID_X - 2.0, 4.0),
        line(GRID_X + 32.0, 1.0),
        line(GRID_X + 127.0, 2.0),
        line(GRID_X + 510.0, 4.0),
    ] {
        let near = |r: &Rect| (r.min - l.min).length() < 1e-3 && (r.max - l.max).length() < 1e-3;
        assert!(lines.iter().any(near), "{l:?}");
    }
}

#[test]
fn a_tapped_hit_pops_in_from_its_centre() {
    let mut rig = Rig::new(empty_clip());
    rig.click(at(1, 47.0));
    // The tap's frame and the next: the click helper's extra frame showed it first, at a = 1, nothing.
    let fill = rig.hit_color(Prop::Velocity, 255, false);
    let size = |rig: &Rig| rig.rects(fill).first().map(|r| (r.center().x, r.width()));
    assert_eq!(size(&rig), Some((GRID_X + 48.0, 0.0)));
    rig.frame_after(1.0 / 60.0, vec![]);
    let (x, w) = size(&rig).unwrap();
    assert_eq!(x, GRID_X + 48.0);
    assert!((w - 32.0 * 0.7199).abs() < 0.05, "{w}");
    rig.frame(vec![]);
    rig.frame(vec![]);
    assert_eq!(size(&rig), Some((GRID_X + 48.0, 32.0)));
}

#[test]
fn the_time_axis_opens_at_4_u_a_beat_pinches_and_scrolls_twice_the_clip() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let key = (CH, 0, Some(1));
    assert_eq!(rig.view.views[&key].k, 0.25);
    // A trackpad pinch ×2 with the pointer half a beat in: twice as wide, that point kept (the rows as they were).
    let p = pos2(GRID_X + 64.0, 200.0);
    rig.frame(vec![egui::Event::PointerMoved(p)]);
    rig.frame(vec![egui::Event::Zoom(2.0)]);
    let v = rig.view.views[&key];
    assert_eq!((v.k, v.x, v.rows.pitch), (0.125, 0.25, 2.0));
    // The scroll stops at twice the 4-beat clip: 8 beats less the 1072 points shown.
    rig.view.views.insert(
        key,
        ClipView {
            k: 0.05,
            x: 100.0,
            ..v
        },
    );
    rig.frame(vec![]);
    let v = rig.view.views[&key];
    assert!((v.x - (8.0 - 1072.0 / 640.0)).abs() < 1e-6, "{}", v.x);
    // The hits follow: tick 0 at the grid's left less the scroll.
    let fill = rig.hit_color(Prop::Velocity, 100, false);
    assert!(rig.rects(fill).is_empty(), "scrolled out of view");
}

#[test]
fn the_graph_lane_lists_three_pages_and_bars_span_each_hit_of_the_current_row() {
    let hits = [drum(0, 64, 64.0), drum(0, 192, 0.0), drum(1, 0, 32.0)];
    let mut rig = Rig::new(notes_lane(&hits));
    rig.view.graph.open = true;
    rig.frame(vec![]);
    for label in ["Velocity", "Panning", "Pitch"] {
        assert!(rig.text_at(label).is_some(), "{label}");
    }
    assert_eq!(rig.text_at("Repeat"), None);
    // The lane is 270 points tall from y 270 (f 0.5 of the 540-point view, whose top margin is 0); its value area starts
    // 4 px lower, right of the header column.
    let fill = rig.hit_color(Prop::Velocity, 100, false);
    let bar = |x0: f32, x1: f32| {
        let top = (540.0 - 266.0 * 100.0 / 255.0_f32).round();
        Rect::from_min_max(pos2(x0, top), pos2(x1, 540.0))
    };
    let bars: Vec<Rect> = rig
        .rects(fill)
        .into_iter()
        .filter(|r| r.top() > 270.0)
        .collect();
    // The Kick's two: over its 64 ticks, and a pixel for the zero-length one. Not the Snare's.
    let x = |tick: f32| GRID_X + tick;
    assert_eq!(bars, [bar(x(64.0), x(128.0)), bar(x(192.0), x(193.0))]);
    // A paint across both at mid-height sets 127, in one undo step, and remembers it.
    rig.drag(pos2(x(74.0), 407.0), &[vec2(30.0, 0.0), vec2(118.0, 0.0)]);
    let vel: Vec<_> = rig
        .events()
        .iter()
        .map(|e| Prop::Velocity.get(e).unwrap())
        .collect();
    assert_eq!(vel, [127, 127, 100]);
    assert_eq!(rig.finals().last(), Some(&true));
    assert_eq!(rig.finals().iter().filter(|f| **f).count(), 1);
    assert_eq!(rig.remembered.velocity, 127);
}

#[test]
fn the_graph_lanes_column_starting_on_the_loop_end_is_still_in_range() {
    // The in-range test is start ≤ L, as the step view's. A one-beat loop: step 4 starts on L, in beat 1 (odd):
    // `moduleFill`; step 5 is past it: `controlFill`.
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].clips[0].loop_len_beats = 1.0;
    let mut rig = Rig::new(channel);
    rig.view.graph.open = true;
    rig.frame(vec![]);
    let colors = theme_by_id(0).colors;
    let left = rig.view.last.unwrap().grid_left();
    let column = |rig: &Rig, step: f32, fill: egui::Color32| {
        (rig.rects(fill).iter()).any(|r| r.top() > 270.0 && r.left() == left + 32.0 * step)
    };
    assert!(column(&rig, 4.0, colors.module_fill.to_color32()));
    assert!(column(&rig, 5.0, colors.control_fill.to_color32()));
}

#[test]
fn the_playhead_line_is_cut_at_the_header_columns_right_edge_and_at_beat_0_shows_in_the_t_gap() {
    // Drawn last with the rows' painter, its t + 1 px would spill over the row headers. As the step view's: cut at the
    // header column's right edge, its left half in the t gap.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.view.playhead = 0;
    rig.frame(vec![]);
    let (text, shadow) = {
        let c = theme_by_id(0).colors;
        let shadow = c.background.to_color32().gamma_multiply(0.3);
        (c.text.to_color32(), shadow)
    };
    let column = 3.0 * 32.0;
    let lines: Vec<(Rect, Rect)> = (rig.shapes.iter())
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if [text, shadow].contains(&r.fill) && r.rect.top() == TOP => {
                Some((r.rect, s.clip_rect))
            }
            _ => None,
        })
        .collect();
    // The shadow and the line, centred on the grid's left (t right of the column), cut at the column.
    assert_eq!(
        lines
            .iter()
            .map(|(r, c)| (crate::ui::timeline::testing::line_x(*r, 1.0), c.left()))
            .collect::<Vec<_>>(),
        [(column + 2.0, column); 2]
    );
}

#[test]
fn the_lanes_knob_drags_f_on_the_editor_below_its_top_margin() {
    // f = (bottom − y)/H, H the whole view (its top margin is 0); the press is in the tab over the lane's top.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.view.graph.open = true;
    rig.frame(vec![]);
    let knob = pos2(44.8, 270.0);
    rig.frame(vec![egui::Event::PointerMoved(knob)]);
    rig.button(knob, PointerButton::Primary, true);
    for y in [250.0, 230.0, 216.0] {
        rig.frame(vec![egui::Event::PointerMoved(pos2(knob.x, y))]);
    }
    rig.button(pos2(knob.x, 216.0), PointerButton::Primary, false);
    assert!(
        (rig.view.graph.f - 0.6).abs() < 1e-6,
        "{}",
        rig.view.graph.f
    );
}

#[test]
fn the_row_menus_file_items_and_the_add_drum_button_ask_the_host() {
    // The step sequencer's row menu (Rotate hidden) and `+` button. `Load Sample`, `Load Drumset` and `Save Drumset`
    // (rows 1–3) and the `+` are the host's to do.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let song = rig.song.clone();
    for (label, action) in [
        ("Load Sample", StepAction::LoadSample { pad: 1 }),
        ("Load Drumset", StepAction::LoadDrumset),
        ("Save Drumset", StepAction::SaveDrumset),
    ] {
        rig.actions.clear();
        // The Snare's header, long-pressed.
        rig.click_with(pos2(48.0, 144.0), PointerButton::Secondary);
        let p = rig.text_at(label).unwrap();
        rig.click(p);
        assert_eq!(rig.actions, [action]);
    }
    // The `+`, 0.75 U below the four rows.
    rig.actions.clear();
    let add = rig.view.last.unwrap().add_button();
    assert_eq!(
        add,
        Rect::from_min_size(pos2(16.0, Y0 + 256.0 + 24.0), vec2(64.0, 64.0))
    );
    rig.click(add.center());
    assert_eq!(rig.actions, [StepAction::AddDrum]);
    assert_eq!(rig.song, song);
}

#[test]
fn the_graph_tab_wins_over_a_cut_off_header() {
    // In both drum modes, where the graph lane's tab overlaps a row header cut off at the rows' bottom, however thin the
    // header's visible part, the tab takes the tap. It ends 2 px into the lane's 4 px gap, clear of the list.
    let mut rig = Rig::new(notes_lane(&[drum(11, 0, 32.0)]));
    let key = (CH, 0, Some(1));
    let view = rig.view.views[&key];
    let rows = RowView {
        pitch: 2.0,
        scroll: (TOP - 20.0) / 32.0,
    };
    rig.view.views.insert(key, ClipView { rows, ..view });
    rig.frame(vec![]);
    // A row header's visible part, at the tab's x (x 18..72).
    let seen = |rig: &Rig, row: usize| {
        let r = rig.ctx.read_response(id().with(("row", row))).unwrap();
        pos2(44.8, r.interact_rect.center().y)
    };
    // Row 8 shows only its top few points, under the closed tab (y 502..540 of 502..542).
    rig.click(seen(&rig, 8));
    assert!(rig.view.graph.open);
    assert_eq!((rig.view.current, rig.view.fold.open), (0, false));
    // The lane's top is at 270: the tab (y 232..272) covers row 3's cut-off header above it ...
    rig.frame(vec![]);
    let row_3 = seen(&rig, 3);
    assert!(232.0 < row_3.y && row_3.y < 270.0, "{row_3:?}");
    rig.click(row_3);
    assert!(!rig.view.graph.open);
    assert_eq!(rig.view.current, 0);
    // ... while the list's first item (`Velocity`, from 274) takes its own tap.
    rig.view.graph.item = 2;
    rig.click(seen(&rig, 8));
    rig.click(pos2(44.8, 280.0));
    assert_eq!((rig.view.graph.open, rig.view.graph.item), (true, 0));
    // Beside the tab the header takes its tap.
    rig.click(row_3);
    rig.click(seen(&rig, 8) + vec2(45.0, 0.0));
    assert_eq!((rig.view.graph.open, rig.view.current), (false, 8));
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
}

impl Rig {
    /// A finger: the touch event plus the pointer events an integration sends with it.
    fn touch(&mut self, phase: egui::TouchPhase, pos: Pos2) {
        let touch = egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let pointer = match phase {
            egui::TouchPhase::Start => vec![egui::Event::PointerMoved(pos), button(true)],
            egui::TouchPhase::Move => vec![egui::Event::PointerMoved(pos)],
            _ => vec![button(false), egui::Event::PointerGone],
        };
        self.frame([vec![touch], pointer].concat());
    }

    /// A second finger: its touch event only, as an integration sends for every finger but the first.
    fn second(&mut self, phase: egui::TouchPhase, pos: Pos2) {
        self.frame(vec![egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(2),
            phase,
            pos,
            force: None,
        }]);
    }

    fn loop_beats(&self) -> f64 {
        self.song.channels[CH].lanes[0].clips[0].loop_len_beats
    }
}

#[test]
fn a_drag_past_the_loop_end_and_back_keeps_the_loop_and_a_pinch_puts_it_back() {
    // A one-bar clip: its end is 512 ticks (x 608) in.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    // Out past the end and back: the loop stays one bar.
    rig.drag(
        at(0, 16.0),
        &[vec2(300.0, 0.0), vec2(600.0, 0.0), vec2(10.0, 0.0)],
    );
    assert_eq!(rig.hits(), [(0, 0.0, 32.0)]);
    assert_eq!(rig.loop_beats(), 4.0);
    // Released past the end, it grows to the bars that hold the hit.
    rig.drag(at(0, 16.0), &[vec2(300.0, 0.0), vec2(600.0, 0.0)]);
    assert_eq!(rig.hits(), [(0, 608.0, 32.0)]);
    assert_eq!(rig.loop_beats(), 8.0);
    // A pinch ends a drag past the end: the hit and the loop go back.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    use egui::TouchPhase::*;
    rig.touch(Start, at(0, 16.0));
    rig.touch(Move, at(0, 316.0));
    rig.touch(Move, at(0, 616.0));
    rig.second(Start, at(2, 300.0));
    assert_eq!(rig.hits(), [(0, 0.0, 32.0)]);
    assert_eq!(rig.loop_beats(), 4.0);
    rig.second(End, at(2, 300.0));
    rig.touch(End, at(0, 616.0));
    assert_eq!((rig.hits(), rig.loop_beats()), (vec![(0, 0.0, 32.0)], 4.0));
}

#[test]
fn a_subdivision_of_0_counts_as_1_and_draws_no_marks_at_no_width() {
    // `TDIV` is stored unchecked, so a file may hold 0: a beat as at d = 1, never 0 wide.
    assert_eq!(beat_w(32.0, 0, 1.0), beat_w(32.0, 1, 1.0));
    assert_eq!(beat_w(32.0, 0, 1.0), 8.0);
    assert_eq!(new_k(0), new_k(1));
    // No width, no marks (and no endless loop to an infinite tick).
    assert_eq!(
        marks(0.0, 512.0, 0.0, 32.0, TimeSig::default(), MARK_GAP),
        []
    );
    assert_eq!(
        marks(
            0.0,
            f64::INFINITY,
            f32::NAN,
            32.0,
            TimeSig::default(),
            MARK_GAP
        ),
        []
    );
}

#[test]
fn a_song_with_subdivision_0_draws_and_takes_a_tap() {
    let mut rig = Rig::new(empty_clip());
    // TDIV byte 0 (the subdivision d) = 0, byte 1 = 4 beats a bar.
    rig.song.time_sig = TimeSig {
        subdivision: 0,
        beats_per_bar: 4,
    };
    rig.view.views.clear();
    rig.frame(vec![]);
    // 4 U a beat at d = 1 is k = 1/16; a tap 47 points in is 47 ticks, the nearest step (a beat at d = 1) is 0. The
    // hit has the remembered length.
    assert_eq!(rig.view.views[&(CH, 0, Some(1))].k, 0.0625);
    rig.click(at(1, 47.0));
    assert_eq!(rig.hits(), [(1, 0.0, 32.0)]);
}

#[test]
fn the_beat_width_follows_tdiv_byte_0_and_the_bar_lines_byte_1() {
    // Beat width = U·d/(4k), d = TDIV byte 0; a new clip opens at 4 U a beat.
    for (d, b) in [(3u8, 3u8), (6, 7)] {
        let mut rig = Rig::new(notes_lane(&[]));
        rig.song.time_sig = TimeSig {
            subdivision: d,
            beats_per_bar: b,
        };
        rig.view.views.clear();
        rig.frame(vec![]);
        assert_eq!(
            rig.view.views[&(CH, 0, None)].k,
            f32::from(d) / 16.0,
            "d {d}"
        );
        // 4 px bar lines (at 1 px a point) every b beats: 128 points a beat from the grid's left. On the grid, under
        // the ruler (whose beat ticks are 2t = 4 points wide).
        let bars: Vec<f32> = (rig.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - 4.0).abs() < 1e-3 && r.rect.top() >= 48.0 =>
                {
                    Some(r.rect.center().x)
                }
                _ => None,
            })
            .filter(|&x| x > GRID_X - 1.0 && x < GRID_X + 1000.0)
            .collect();
        let want = |n: f32| GRID_X + n * f32::from(b) * 128.0;
        assert!(
            bars.iter().any(|&x| (x - want(1.0)).abs() < 0.5),
            "b {b}: {bars:?}"
        );
        assert!(
            !bars.iter().any(|&x| (x - GRID_X - 512.0).abs() < 0.5),
            "b {b}: {bars:?}"
        );
    }
}

#[test]
fn fill_and_clear_fit_the_loop() {
    // The loop is fitted after every clip edit, the row menu's too.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0), drum(1, 600, 32.0)]));
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 8.0;
    let clip = rig.clip;
    rig.view
        .apply(&mut rig.song, CH, (0, clip), &StepAction::Clear { pad: 1 });
    assert_eq!(rig.song.channels[CH].lanes[0].clips[0].loop_len_beats, 4.0);
    // Fill over a three-bar loop: its quarter-note hits reach 12 beats, which fit in 16.
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 12.0;
    let fill = StepAction::Fill { pad: 0, every: 4 };
    rig.view.apply(&mut rig.song, CH, (0, clip), &fill);
    assert_eq!(rig.song.channels[CH].lanes[0].clips[0].loop_len_beats, 16.0);
}

/// The row menu's Fill reads d from TDIV byte 0, not byte 1, the beats per bar.
#[test]
fn the_row_menus_fill_spaces_hits_by_tdiv_byte_0() {
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.song.time_sig = TimeSig {
        subdivision: 3,
        beats_per_bar: 4,
    };
    let clip = rig.clip;
    let fill = StepAction::Fill { pad: 0, every: 4 };
    rig.view.apply(&mut rig.song, CH, (0, clip), &fill);
    assert_eq!(
        rig.hits(),
        [(0, 0.0, 128.0), (0, 171.0, 128.0), (0, 341.0, 128.0)]
    );
}

#[test]
fn a_drag_or_pinch_that_puts_the_hits_back_leaves_a_long_loop_as_it_was() {
    // Clip::fit_loop would shrink four bars holding one hit in bar 1 to one bar: only an edit fits the loop.
    let long = || {
        let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
        rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 16.0;
        rig.frame(vec![]);
        rig
    };
    let mut rig = long();
    rig.drag(
        at(0, 16.0),
        &[vec2(300.0, 0.0), vec2(600.0, 0.0), vec2(10.0, 0.0)],
    );
    assert_eq!((rig.hits(), rig.loop_beats()), (vec![(0, 0.0, 32.0)], 16.0));
    // The release still closes the undo step the moves opened (it records none: the song is as it was).
    assert_eq!(rig.finals(), [false, false, false, true]);
    // A pinch puts the hit back.
    let mut rig = long();
    use egui::TouchPhase::*;
    rig.touch(Start, at(0, 16.0));
    rig.touch(Move, at(0, 316.0));
    rig.second(Start, at(2, 300.0));
    rig.second(End, at(2, 300.0));
    rig.touch(End, at(0, 316.0));
    assert_eq!((rig.hits(), rig.loop_beats()), (vec![(0, 0.0, 32.0)], 16.0));
    assert_eq!(rig.finals().last(), Some(&true));
    // A drag that moves the hit fits the loop: one bar holds it.
    let mut rig = long();
    rig.drag(at(0, 16.0), &[vec2(300.0, 0.0)]);
    assert_eq!(
        (rig.hits(), rig.loop_beats()),
        (vec![(0, 288.0, 32.0)], 4.0)
    );
}

#[test]
fn the_free_grids_wheel_zooms_the_time_on_the_ruler_the_rows_on_the_headers_and_scrolls_on_the_grid()
 {
    // The playlist's zones, one raw wheel turn each.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 64.0;
    let wheel = |rig: &mut Rig, at: Pos2, delta: Vec2| {
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers: Modifiers::NONE,
            },
        ]);
    };
    let view = |rig: &Rig| *rig.view.views.values().next().unwrap();
    let start = view(&rig);
    // The ruler: the time (k 1/1.25 of what it was: a beat 1.25 times as wide), the beat under the pointer kept.
    let p = pos2(600.0, 24.0);
    let beat = |v: ClipView| v.x + f64::from((p.x - GRID_X) / edit::beat_w(32.0, 4, v.k));
    wheel(&mut rig, p, vec2(0.0, 200.0));
    let v = view(&rig);
    assert!((v.k - start.k * 0.8).abs() < 1e-6, "{v:?}");
    assert!((beat(v) - beat(start)).abs() < 1e-4);
    assert_eq!(v.rows, start.rows);
    // The headers: the rows (pitch 2 U → 2.5 U).
    wheel(&mut rig, pos2(48.0, 200.0), vec2(0.0, 200.0));
    let w = view(&rig);
    assert!((w.rows.pitch - 2.5).abs() < 1e-5, "{w:?}");
    assert_eq!((w.k, w.x), (v.k, v.x));
    // The grid: it scrolls (the time here: one pad's row fits the view), the zooms as they were.
    wheel(&mut rig, pos2(600.0, 150.0), vec2(-40.0, -32.0));
    let s = view(&rig);
    assert!(s.x > w.x, "{s:?}");
    assert_eq!((s.k, s.rows.pitch), (w.k, w.rows.pitch));
}

/// The other gestures the remembered-hits guard covers: a graph paint whose release, pinch, or closed graph lane leaves
/// the hits as they were does not fit the loop; one that changed them does. A release or pinch that leaves them is a
/// touch that changed nothing ([`StepAction::GraphCancel`], whose own undo re-syncs).
#[test]
fn a_graph_paint_that_puts_the_values_back_leaves_a_long_loop_as_it_was() {
    // Four bars holding one Kick hit in bar 1, the graph lane open on Velocity: y 435 paints the hit's own 100,
    // y 407 paints 127, at x 110 (tick 12, inside the hit).
    let long = || {
        let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
        rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 16.0;
        rig.view.graph.open = true;
        rig.frame(vec![]);
        rig
    };
    let (own, mid) = (pos2(110.0, 435.0), pos2(110.0, 407.0));
    let velocity = |rig: &Rig| Prop::Velocity.get(&rig.events()[0]);
    let unchanged = |rig: &Rig| matches!(rig.actions.last(), Some(StepAction::GraphCancel(_)));
    // Released after painting back.
    let mut rig = long();
    rig.drag(own, &[mid - own, Vec2::ZERO]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert_eq!(rig.finals(), [false, false]);
    assert!(unchanged(&rig));
    // A second finger ends the paint, putting back what it painted.
    let mut rig = long();
    use egui::TouchPhase::*;
    rig.touch(Start, own);
    rig.touch(Move, mid);
    assert_eq!(velocity(&rig), Some(127));
    rig.second(Start, at(2, 300.0));
    rig.second(End, at(2, 300.0));
    rig.touch(End, mid);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert!(unchanged(&rig));
    // The graph lane closes during a paint that went back.
    let mut rig = long();
    rig.frame(vec![egui::Event::PointerMoved(own)]);
    rig.button(own, PointerButton::Primary, true);
    rig.frame(vec![egui::Event::PointerMoved(mid)]);
    rig.frame(vec![egui::Event::PointerMoved(own)]);
    rig.view.graph.open = false;
    rig.frame(vec![]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(100), 16.0));
    assert_eq!(rig.finals().last(), Some(&true));
    rig.release(own);
    // A paint that changed the velocity fits the loop: one bar holds the hit.
    let mut rig = long();
    rig.drag(own, &[mid - own]);
    assert_eq!((velocity(&rig), rig.loop_beats()), (Some(127), 4.0));
    // A pinch before any paint (the touch-down at the hit's own value): a touch that changed nothing, once.
    let mut rig = long();
    rig.touch(Start, own);
    rig.second(Start, at(2, 300.0));
    assert!(unchanged(&rig));
    rig.second(End, at(2, 300.0));
    rig.touch(End, own);
    let cancels = rig
        .actions
        .iter()
        .filter(|a| matches!(a, StepAction::GraphCancel(_)));
    assert_eq!((cancels.count(), rig.loop_beats()), (1, 16.0));
}

#[test]
fn a_trackpad_pinch_over_the_free_grid_zooms_the_time_only() {
    // The playlist's pinch zone, zoom x.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let view = |rig: &Rig| *rig.view.views.values().next().unwrap();
    let before = view(&rig);
    rig.frame(vec![
        egui::Event::PointerMoved(pos2(600.0, 150.0)),
        egui::Event::Zoom(2.0),
    ]);
    let after = view(&rig);
    assert!((after.k - before.k / 2.0).abs() < 1e-6, "{after:?}");
    assert_eq!(after.rows, before.rows);
}

#[test]
fn the_drum_editors_zones_are_round_21s() {
    // An 800 × 448 editor, the drum mixer half out (HW = 3 U + 0.5·12 U = 288 points, GL = 288 + ceil(U/16) = 290),
    // the ruler TOP.
    let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
    let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 448.0));
    let l = Layout::new(rect, 4, 1, &m, 192.0, RowView::default());
    let r = |x0, y0, x1, y1| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let z = step_seq::zones(&l);
    // Touch scroll y: the full width below the ruler; touch scroll x: the grid columns from the ruler's top.
    assert_eq!(
        z.touch,
        [r(290.0, 0.0, 800.0, 448.0), r(0.0, TOP, 800.0, 448.0)]
    );
    // Wheel zoom x over the ruler band, zoom y over the header column; the grid scrolls and takes the pinch.
    assert_eq!(
        z.wheel_zoom,
        [r(290.0, 0.0, 800.0, TOP), r(0.0, TOP, 288.0, 448.0)]
    );
    assert_eq!(
        (z.wheel_scroll, z.pinch),
        (r(290.0, TOP, 800.0, 448.0), r(290.0, TOP, 800.0, 448.0))
    );
}

#[test]
fn one_finger_scrolls_the_rows_from_the_headers_and_both_ways_on_the_grid_and_two_zoom_both() {
    // The drum editor's zones on the scroll and zoom laws.
    use egui::TouchPhase::{End, Move, Start};
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    rig.song.channels[CH].lanes[0].clips[0].loop_len_beats = 64.0;
    let view = |rig: &Rig| *rig.view.views.values().next().unwrap();
    // Rows 100/24 U tall (the wheel over the headers): the 4 pads and 1.5 rows of room outgrow the rows' area.
    for _ in 0..4 {
        rig.frame(vec![
            egui::Event::PointerMoved(pos2(48.0, 100.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 300.0),
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    let v = view(&rig);
    assert!((v.rows.pitch - 100.0 / 24.0).abs() < 1e-5, "{v:?}");
    // On the header column, 60 points up and 60 sideways: the rows follow, the time does not.
    rig.touch(Start, pos2(48.0, 300.0));
    rig.touch(Move, pos2(108.0, 240.0));
    rig.frame(vec![]);
    rig.touch(End, pos2(108.0, 240.0));
    let w = view(&rig);
    assert!(
        (w.rows.scroll - v.rows.scroll - 60.0 / 32.0).abs() < 1e-4,
        "{w:?}"
    );
    assert_eq!(w.x, v.x);
    // On empty grid: both.
    rig.touch(Start, pos2(600.0, 300.0));
    rig.touch(Move, pos2(540.0, 280.0));
    rig.frame(vec![]);
    rig.touch(End, pos2(540.0, 280.0));
    let s = view(&rig);
    assert!((s.x - w.x - 60.0 / 128.0).abs() < 1e-6, "{s:?}");
    assert!(
        (s.rows.scroll - w.rows.scroll - 20.0 / 32.0).abs() < 1e-4,
        "{s:?}"
    );
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    // Two fingers closing 300 → 200 points sideways and 150 → 100 down: the time and the rows zoom out 1.5 times.
    rig.touch(Start, pos2(600.0, 300.0));
    rig.second(Start, pos2(900.0, 450.0));
    rig.second(Move, pos2(800.0, 400.0));
    rig.second(End, pos2(800.0, 400.0));
    rig.touch(End, pos2(600.0, 300.0));
    let p = view(&rig);
    assert!((p.k - s.k * 1.5).abs() < 1e-5, "{p:?}");
    assert!((p.rows.pitch - s.rows.pitch / 1.5).abs() < 1e-4, "{p:?}");
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
}

#[test]
fn another_clip_takes_no_fling_from_the_last_one() {
    // A fling of one clip's time does not move the next clip's view.
    let mut channel = notes_lane(&[drum(0, 0, 32.0)]);
    channel.lanes[0].add_clip(4096, 64.0);
    channel.lanes[0].clips[0].loop_len_beats = 64.0;
    let mut rig = Rig::new(channel);
    let second = |rig: &Rig| rig.view.views.get(&(CH, 0, Some(2))).map(|v| v.x);
    let from = at(3, 700.0);
    rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(from)]);
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
    );
    for k in 1..=3 {
        let to = from - vec2(60.0 * k as f32, 0.0);
        rig.frame_after(1.0 / 60.0, vec![egui::Event::PointerMoved(to)]);
    }
    let to = from - vec2(180.0, 0.0);
    rig.frame_after(
        1.0 / 60.0,
        vec![egui::Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    let view = |rig: &Rig| rig.view.views[&(CH, 0, Some(1))].x;
    let flying = view(&rig);
    rig.frame_after(1.0 / 60.0, vec![]);
    assert_ne!(view(&rig), flying, "the first clip flings");
    rig.clip = Some(2);
    for _ in 0..10 {
        rig.frame_after(1.0 / 60.0, vec![]);
    }
    assert_eq!(second(&rig), Some(0.0));
}

// The keymap's editing and zoom actions (`ui/edit_keys.rs`).

impl Rig {
    /// A press of `key` with `mods` and its release, in one frame.
    fn key(&mut self, key: egui::Key, mods: Modifiers) {
        self.frame(crate::ui::edit_keys::tests::tap(key, mods).into());
    }
}

#[test]
fn the_editing_keys_copy_cut_delete_and_paste_hits_at_the_touched_beat_one_undo_step_each() {
    let cmd = Modifiers::COMMAND;
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 64.0), drum(1, 64, 32.0)]));
    rig.click(at(1, 80.0));
    rig.key(egui::Key::C, cmd);
    assert_eq!(hit_clipboard(&rig.ctx), [drum(1, 64, 32.0)]);
    assert!(rig.actions.is_empty(), "copy edits nothing");
    // The floating row hides (from the next frame: the keys come last in the frame).
    rig.frame(vec![]);
    assert_eq!(rig.text_at("Copy"), None);
    // Touching the Kick at beat 1.5 puts the cursor there; paste lands the copy on the nearest snap unit.
    rig.click(at(0, 50.0));
    rig.key(egui::Key::V, cmd);
    assert_eq!(rig.finals(), [true]);
    assert_eq!(
        rig.hits(),
        [(0, 0.0, 64.0), (1, 64.0, 32.0), (1, 64.0, 32.0)]
    );
    // Ctrl+X: the pasted (selected) hit to the clipboard and gone; Delete: the selection gone.
    rig.key(egui::Key::X, cmd);
    assert_eq!(rig.hits(), [(0, 0.0, 64.0), (1, 64.0, 32.0)]);
    rig.click(at(0, 10.0));
    rig.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(rig.hits(), [(1, 64.0, 32.0)]);
    assert_eq!(rig.finals(), [true; 3]);
    // Nothing selected: Delete edits nothing.
    rig.key(egui::Key::Delete, Modifiers::NONE);
    assert_eq!(rig.finals(), [true; 3]);
}

#[test]
fn enter_adds_a_hit_at_the_cursor_and_escape_deselects() {
    let mut rig = Rig::new(notes_lane(&[drum(2, 256, 32.0)]));
    rig.click(at(2, 260.0));
    assert_eq!(rig.selected(), [true]);
    rig.key(egui::Key::Escape, Modifiers::NONE);
    assert_eq!(rig.selected(), [false]);
    assert!(rig.actions.is_empty());
    // The touched hit holds the cell: Enter adds nothing.
    rig.key(egui::Key::Enter, Modifiers::NONE);
    assert!(rig.actions.is_empty());
    // Deleted, the cell is empty: Enter adds a hit on the touched row, at its beat rounded to the snap unit.
    rig.click(at(2, 260.0));
    rig.key(egui::Key::Delete, Modifiers::NONE);
    rig.key(egui::Key::Enter, Modifiers::NONE);
    assert_eq!(rig.finals(), [true, true]);
    // A new hit, as a tap's: no re-sync.
    let quiet = matches!(
        rig.actions.last(),
        Some(StepAction::Hits { quiet: true, .. })
    );
    assert!(quiet, "{:?}", rig.actions.last());
    assert_eq!(rig.hits(), [(2, 256.0, 32.0)]);
}

#[test]
fn a_view_hidden_mid_drag_ends_the_drag_and_drags_nothing_back_on_screen() {
    // The host closes the popups of a drum editor it does not draw; a gesture under way goes with them.
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let from = at(0, 16.0);
    rig.frame(vec![egui::Event::PointerMoved(from)]);
    rig.button(from, PointerButton::Primary, true);
    rig.frame(vec![egui::Event::PointerMoved(from + vec2(64.0, 0.0))]);
    assert_eq!(rig.hits(), [(0, 64.0, 32.0)]);
    // It reports that the drag it ended had edits out (the host closes their undo step), once.
    assert!(rig.view.close_popups());
    assert!(matches!(rig.view.gesture, Gesture::Idle));
    assert!(rig.view.hits_from.is_none());
    assert!(!rig.view.close_popups());
    // Back on screen with the finger still down: its moves and its lift edit nothing.
    rig.actions.clear();
    let to = from + vec2(128.0, 0.0);
    rig.frame(vec![egui::Event::PointerMoved(to)]);
    rig.button(to, PointerButton::Primary, false);
    rig.frame(vec![]);
    assert_eq!(rig.hits(), [(0, 64.0, 32.0)]);
    assert!(rig.finals().is_empty(), "{:?}", rig.finals());
}

#[test]
fn the_shared_long_press_time_and_calm_zone_decide_a_touch_on_empty_grid() {
    // `gesture::LONG_PRESS_S` and `CALM_U` (0.3 s; 0.5 U, 16 points at U = 32): released at 0.28 s, or moved 0.45 U,
    // a press on an empty cell is a tap (a hit); at 0.32 s it is a long press, 0.55 U out a scroll: nothing.
    let hits_after = |secs: f64, by: f32| {
        // A lane with an empty clip: the tap's hit needs no new clip.
        let mut lane = notes_lane(&[]);
        lane.lanes[0].add_clip(0, 4.0);
        let mut rig = Rig::new(lane);
        let p = at(2, 200.0);
        rig.frame(vec![egui::Event::PointerMoved(p)]);
        rig.button(p, PointerButton::Primary, true);
        let mut to = p;
        let mut held = 0.0;
        if by != 0.0 {
            to = p + vec2(by, 0.0);
            rig.frame(vec![egui::Event::PointerMoved(to)]);
            held = 0.1;
        }
        let up = egui::Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        };
        rig.frame_after(secs - held, vec![up]);
        rig.frame(vec![]);
        rig.hits().len()
    };
    assert_eq!(hits_after(0.28, 0.0), 1);
    assert_eq!(hits_after(0.32, 0.0), 0);
    assert_eq!(hits_after(0.2, 0.45 * 32.0), 1);
    assert_eq!(hits_after(0.2, 0.55 * 32.0), 0);
}

/// The play box fades as the drum editor's (`timeline::knob_shown`, through `timeline::PlayheadKnob`): out while
/// playing, so that only the line shows, and back in once stopped.
#[test]
fn the_play_box_fades_out_while_playing_and_back_in_stopped() {
    let menu = theme_by_id(0).colors.menu_fill.to_color32();
    let mut rig = Rig::new(notes_lane(&[drum(0, 0, 32.0)]));
    let disc = |rig: &Rig| {
        crate::ui::timeline::testing::play_boxes(&rig.shapes)
            .next()
            .map(|b| b.fill)
    };
    rig.view.playhead = 0;
    for (playing, fill) in [(false, Some(menu)), (true, None), (false, Some(menu))] {
        rig.view.playing = playing;
        for _ in 0..40 {
            rig.frame(vec![]);
        }
        assert_eq!(disc(&rig), fill, "playing {playing}");
    }
}
