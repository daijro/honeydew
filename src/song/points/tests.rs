use super::*;
use crate::song::{Lane, LaneKind};

const MAX: u16 = 65534;

fn pt(tick: u32, value: u16) -> Point {
    Point {
        tick,
        value,
        ..Default::default()
    }
}

fn clip(kind: LaneKind) -> Clip {
    Lane::new(kind).add_clip(0, 4.0).clone()
}

fn auto(pos: u32, value: u16, control: bool) -> Event {
    Event::Automation {
        pos,
        value: unit(value),
        control,
    }
}

fn env(pos: u32, env_param: i16, value: u16, control: bool) -> Event {
    Event::Envelope {
        pos,
        env_param,
        value: unit(value),
        control,
    }
}

/// The ramp the region fixtures cut: 0 at 0, 1000 at 256, 3000 at 512.
fn ramp() -> Vec<Point> {
    vec![pt(0, 0), pt(256, 1000), pt(512, 3000)]
}

#[test]
fn every_u16_survives_the_model_float() {
    for v in 0..=u16::MAX {
        assert_eq!(raw(unit(v)), v);
    }
    assert_eq!(
        (raw(0.0), raw(1.0), raw(-1.0), raw(2.0)),
        (0, 65535, 0, 65535)
    );
}

#[test]
fn the_keys_full_scale() {
    assert_eq!(PointKey::Automation.max(), 65534);
    assert_eq!(PointKey::Envelope(1).max(), 65535);
}

#[test]
fn points_are_the_keys_control_points_only() {
    let mut c = clip(LaneKind::Automation);
    c.events = vec![
        auto(0, 1000, true),
        auto(10, 1200, false),
        auto(20, 65535, true),
    ];
    assert_eq!(
        points(&c, PointKey::Automation),
        [pt(0, 1000), pt(20, 65535)]
    );
    let mut c = clip(LaneKind::Audio);
    c.events = vec![
        env(0, 0, 100, true),
        env(5, 1, 200, true),
        env(9, 0, 300, false),
    ];
    assert_eq!(points(&c, PointKey::Envelope(0)), [pt(0, 100)]);
    assert_eq!(points(&c, PointKey::Envelope(1)), [pt(5, 200)]);
    assert_eq!(points(&c, PointKey::Envelope(2)), []);
}

#[test]
fn a_full_ramp_rebuilds_127_steps_once() {
    let mut c = clip(LaneKind::Automation);
    // Unsorted on purpose: set_points sorts.
    set_points(&mut c, PointKey::Automation, vec![pt(512, MAX), pt(0, 0)]);
    let steps = |c: &Clip| {
        (c.events.iter())
            .filter(|e| matches!(e, Event::Automation { control: false, .. }))
            .count()
    };
    assert_eq!((c.events.len(), steps(&c)), (129, 127));
    assert!(c.events.is_sorted_by_key(Event::pos));
    assert_eq!(c.events[0], auto(0, 0, true));
    // Level 1 = trunc(511.984375) at 512·1/127 ticks.
    assert_eq!(c.events[1], auto(4, 511, false));
    assert_eq!(c.events[128], auto(512, MAX, true));
    // Generated steps never accumulate.
    let once = c.events.clone();
    for _ in 0..3 {
        let pts = points(&c, PointKey::Automation);
        set_points(&mut c, PointKey::Automation, pts);
    }
    assert_eq!(c.events, once);
    // Descending: as many steps, values in reverse order.
    let mut down = clip(LaneKind::Automation);
    set_points(
        &mut down,
        PointKey::Automation,
        vec![pt(0, MAX), pt(512, 0)],
    );
    let values = |c: &Clip| -> Vec<u16> {
        (c.events.iter())
            .filter_map(|e| match *e {
                Event::Automation {
                    value,
                    control: false,
                    ..
                } => Some(raw(value)),
                _ => None,
            })
            .collect()
    };
    let mut rev = values(&down);
    rev.reverse();
    assert_eq!(rev, values(&c));
    // One level apart (63 → 64): no step at all.
    set_points(
        &mut c,
        PointKey::Automation,
        vec![pt(0, 32256), pt(512, 32768)],
    );
    assert_eq!((c.events.len(), steps(&c)), (2, 0));
}

#[test]
fn set_points_keeps_the_other_keys_events() {
    let mut c = clip(LaneKind::Audio);
    set_points(
        &mut c,
        PointKey::Envelope(0),
        vec![pt(0, 0), pt(128, 65535)],
    );
    let level = c.events.clone();
    assert_eq!(level.len(), 2 + 127);
    set_points(&mut c, PointKey::Envelope(1), vec![pt(64, 9)]);
    assert_eq!(
        points(&c, PointKey::Envelope(0)),
        [pt(0, 0), pt(128, 65535)]
    );
    assert_eq!(points(&c, PointKey::Envelope(1)), [pt(64, 9)]);
    let others: Vec<Event> = (c.events.iter())
        .filter(|e| matches!(e, Event::Envelope { env_param: 0, .. }))
        .copied()
        .collect();
    assert_eq!(others, level);
    assert!(c.events.is_sorted_by_key(Event::pos));
    // An envelope's steps carry its env_param.
    assert!(c.events.contains(&env(1, 0, 511, false)));
}

#[test]
fn an_imported_65535_survives_where_not_edited() {
    let mut c = clip(LaneKind::Automation);
    c.events = vec![auto(0, 65535, true), auto(128, 0, true)];
    let pts = points(&c, PointKey::Automation);
    assert_eq!(pts[0], pt(0, 65535));
    // Moving only the other point, in time and rows, leaves it.
    let (moved, _) = moved(&pts, &[false, true], 128.0, -3.0, None, MAX).unwrap();
    assert_eq!(moved[0], pt(0, 65535));
    set_points(&mut c, PointKey::Automation, moved);
    assert_eq!(c.events[0], auto(0, 65535, true));
    // Moved in time only, its value stays too.
    let (moved, _) = super::moved(&pts, &[true, false], 64.0, 0.0, None, MAX).unwrap();
    assert_eq!(moved[0], pt(64, 65535));
}

#[test]
fn rows_and_values() {
    assert_eq!(row(MAX, MAX), 0.0);
    assert_eq!(row(0, MAX), 126.0);
    assert_eq!(row(32767, MAX), 63.0);
    assert_eq!(row(65535, 65535), 0.0);
    assert!(row(65535, MAX) < 0.0);
    // Every value is the value of its own row.
    for max in [MAX, 65535] {
        for v in 0..=max {
            assert_eq!(value_at_row(row(v, max), max), v, "{v} of {max}");
        }
    }
    assert_eq!(value_at_row(0.0, MAX), MAX);
    assert_eq!(value_at_row(0.0, 65535), 65535);
    assert_eq!(value_at_row(-5.0, MAX), MAX);
    assert_eq!(value_at_row(126.0, MAX), 0);
    assert_eq!(value_at_row(127.0, MAX), 0);
    // trunc(65534 − 0.5·65534/126) = trunc(65273.94).
    assert_eq!(value_at_row(0.5, MAX), 65273);
}

#[test]
fn create_snaps_rounds_and_stays_at_beat_zero() {
    let pts = vec![pt(0, 1), pt(64, 2)];
    let (new, sel) = create(&pts, 32.0, 0.0, None, MAX);
    assert_eq!(new, [pt(0, 1), pt(32, MAX), pt(64, 2)]);
    assert_eq!(sel, [false, true, false]);
    // The bottom of the value area (r = 127, a row below the last) and the last row are both 0.
    assert_eq!(create(&[], 0.0, 127.0, None, MAX).0, [pt(0, 0)]);
    assert_eq!(create(&[], 0.0, 126.0, None, MAX).0, [pt(0, 0)]);
    assert_eq!(create(&[], 0.0, 0.0, None, 65535).0, [pt(0, 65535)]);
    let tick = |t: f64, snap| create(&[], t, 0.0, snap, MAX).0[0].tick;
    // Snap on (a 32-tick unit): round to the nearest unit, halves away from zero.
    assert_eq!((tick(47.0, Some(32.0)), tick(48.0, Some(32.0))), (32, 64));
    // Snap off: the nearest tick.
    assert_eq!((tick(47.4, None), tick(47.6, None)), (47, 48));
    // Never before beat 0.
    assert_eq!((tick(-20.0, None), tick(-20.0, Some(32.0))), (0, 0));
    // A new point goes after the points at its tick.
    let (new, sel) = create(&pts, 64.0, 126.0, None, MAX);
    assert_eq!((new[2], sel), (pt(64, 0), vec![false, false, true]));
}

#[test]
fn a_drag_snaps_the_boxs_left_beat_and_moves_whole_rows() {
    let pts = vec![pt(0, 0), pt(128, 32767), pt(256, MAX)];
    let sel = [false, true, false];
    // 128 + 70 = 198 → round(6.19) units of 32 = 192; 1.4 rows → 1: row 64.
    let (new, s) = moved(&pts, &sel, 70.0, 1.4, Some(32.0), MAX).unwrap();
    assert_eq!(new, [pt(0, 0), pt(192, 32246), pt(256, MAX)]);
    assert_eq!(s, sel);
    // Snapped back onto its own beat with less than half a row: nothing changed.
    assert_eq!(moved(&pts, &sel, 10.0, 0.4, Some(32.0), MAX), None);
    // Past another point: re-sorted with the selection.
    let (new, s) = moved(&pts, &sel, 200.0, 0.0, None, MAX).unwrap();
    assert_eq!(new, [pt(0, 0), pt(256, MAX), pt(328, 32767)]);
    assert_eq!(s, [false, false, true]);
}

#[test]
fn a_drag_keeps_the_box_at_beat_zero_and_inside_the_rows() {
    let pts = vec![pt(64, MAX), pt(96, 32767), pt(500, 7)];
    let sel = [true, true, false];
    // The box's left edge stops at beat 0 and the group keeps its offsets, snap on or off.
    for snap in [None, Some(32.0)] {
        let (new, _) = moved(&pts, &sel, -100.0, 0.0, snap, MAX).unwrap();
        assert_eq!(new[..2], [pt(0, MAX), pt(32, 32767)]);
    }
    // Row 0 is the top: up does nothing.
    assert_eq!(moved(&pts, &sel, 0.0, -3.0, None, MAX), None);
    // Down, the lowest point stops at row 126 (value 0) and the other moves as far: 63 rows.
    let (new, _) = moved(&pts, &sel, 0.0, 100.0, None, MAX).unwrap();
    assert_eq!(new[..2], [pt(64, 32767), pt(96, 0)]);
    // A box that starts between rows reaches the top exactly: 65000 lies 1.03 rows down.
    let (new, _) = moved(&[pt(0, 65000)], &[true], 0.0, -5.0, None, MAX).unwrap();
    assert_eq!(new, [pt(0, MAX)]);
    // Envelopes: rows of value/65535.
    let (new, _) = moved(&[pt(0, 0)], &[true], 0.0, -126.0, None, 65535).unwrap();
    assert_eq!(new, [pt(0, 65535)]);
}

#[test]
fn the_right_edge_handle_scales_about_the_left_edge() {
    let pts = vec![pt(128, 1), pt(256, 2), pt(384, 3)];
    let all = [true; 3];
    // The right edge 384 + 100 snaps to 512: ×1.5 about 128.
    let (new, _) = scaled(&pts, &all, 100.0, Some(128.0)).unwrap();
    assert_eq!(new, [pt(128, 1), pt(320, 2), pt(512, 3)]);
    // Never left of the left edge.
    let (new, _) = scaled(&pts, &all, -1000.0, None).unwrap();
    assert_eq!(new, [pt(128, 1), pt(128, 2), pt(128, 3)]);
    // Snapped back onto itself, one point, or a box without width: nothing.
    assert_eq!(scaled(&pts, &all, 10.0, Some(128.0)), None);
    assert_eq!(scaled(&pts, &[false, true, false], 100.0, None), None);
    assert_eq!(scaled(&[pt(5, 1), pt(5, 2)], &[true; 2], 100.0, None), None);
    // Only the selection scales.
    let (new, s) = scaled(&pts, &[true, true, false], 128.0, None).unwrap();
    assert_eq!(new, [pt(128, 1), pt(384, 2), pt(384, 3)]);
    assert_eq!(s, [true, true, false]);
}

#[test]
fn the_marquee_takes_beats_in_range_and_one_row_bands() {
    // Rows 0, 63 and 126.
    let pts = vec![pt(0, MAX), pt(128, 32767), pt(256, 0)];
    assert_eq!(
        marquee(&pts, [0.0, 128.0], [62.5, 70.0], MAX),
        [false, true, false]
    );
    // In either order; the band [63, 64] reaches 64.
    assert_eq!(
        marquee(&pts, [128.0, 0.0], [64.0, 64.0], MAX),
        [false, true, false]
    );
    assert_eq!(marquee(&pts, [0.0, 128.0], [64.01, 70.0], MAX), [false; 3]);
    // Row 0's band is [0, 1].
    assert_eq!(
        marquee(&pts, [0.0, 256.0], [0.5, 0.6], MAX),
        [true, false, false]
    );
    assert_eq!(marquee(&pts, [0.0, 256.0], [0.0, 127.0], MAX), [true; 3]);
    assert_eq!(marquee(&pts, [1.0, 127.0], [0.0, 127.0], MAX), [false; 3]);
}

#[test]
fn copy_and_paste_keep_offsets_values_and_shapes() {
    let square = Shape {
        kind: ShapeKind::Square,
        tension: 30,
    };
    let shaped = Point {
        shape: square,
        ..pt(128, MAX)
    };
    let pts = vec![pt(0, 5), shaped, pt(192, 0)];
    let clipboard = copy(&pts, &[false, true, true], MAX);
    assert_eq!(
        clipboard,
        [(1.0, 1.0, square), (1.5, 0.0, Shape::default())]
    );
    // 300 snaps to 288 (units of 32).
    let (new, sel) = paste(&pts, &clipboard, 300.0, Some(32.0), MAX);
    let pasted = Point {
        shape: square,
        ..pt(288, MAX)
    };
    assert_eq!(new, [pt(0, 5), shaped, pt(192, 0), pasted, pt(352, 0)]);
    let pts = vec![pt(0, 5), pt(128, MAX), pt(192, 0)];
    let clipboard = copy(&pts, &[false, true, true], MAX);
    assert_eq!(sel, [false, false, false, true, true]);
    assert_eq!(
        paste(&[], &clipboard, -40.0, None, MAX).0,
        [pt(0, MAX), pt(64, 0)]
    );
    assert_eq!(paste(&[], &clipboard, 3.4, None, MAX).0[0], pt(3, MAX));
    // Full scale to full scale across keys; an imported 65535 pastes back as it was.
    let full = copy(&[pt(0, 65535)], &[true], 65535);
    assert_eq!(paste(&[], &full, 0.0, None, MAX).0, [pt(0, MAX)]);
    let imported = copy(&[pt(0, 65535)], &[true], MAX);
    assert_eq!(paste(&[], &imported, 0.0, None, MAX).0, [pt(0, 65535)]);
    // Pasted points go after the points at their tick.
    let (new, sel) = paste(&pts, &clipboard, 128.0, None, MAX);
    assert_eq!(new[1..3], [pt(128, MAX), pt(128, MAX)]);
    assert_eq!(sel, [false, false, true, false, true]);
}

#[test]
fn a_break_uses_the_point_there_or_interpolates() {
    let mut pts = ramp();
    assert_eq!(break_at(&mut pts, 0), Some(0));
    assert_eq!(break_at(&mut pts, 512), Some(2));
    assert_eq!(pts, ramp());
    assert_eq!(break_at(&mut pts, 128), Some(1));
    assert_eq!(pts[1], pt(128, 500));
    // On a shaped segment: the value on its curve, and its shape on both sides.
    let square = Shape {
        kind: ShapeKind::Square,
        tension: 50,
    };
    let mut held = vec![
        Point {
            shape: square,
            ..pt(0, 0)
        },
        pt(512, 2000),
    ];
    assert_eq!(break_at(&mut held, 128), Some(1));
    assert_eq!(
        held[1],
        Point {
            shape: square,
            ..pt(128, 0)
        }
    );
    // Between a jump's right side and the next point.
    let mut jump = vec![pt(0, 0), pt(256, 1000), pt(256, 2000), pt(512, 3000)];
    assert_eq!(break_at(&mut jump, 384), Some(3));
    assert_eq!(jump[3], pt(384, 2500));
    // The first point of a jump.
    assert_eq!(break_at(&mut jump, 256), Some(1));
    // None before the first point or after the last.
    let mut late = vec![pt(64, 1), pt(128, 2)];
    assert_eq!(
        (break_at(&mut late, 10), break_at(&mut late, 200)),
        (None, None)
    );
    assert_eq!(late, [pt(64, 1), pt(128, 2)]);
    assert_eq!(break_at(&mut Vec::new(), 0), None);
    // Fractional: 333.3 and 666.7 round to the nearest value, rising and falling.
    let mut up = vec![pt(0, 0), pt(3, 1000)];
    assert_eq!(
        (break_at(&mut up, 1), break_at(&mut up, 2)),
        (Some(1), Some(2))
    );
    assert_eq!(up, [pt(0, 0), pt(1, 333), pt(2, 667), pt(3, 1000)]);
    let mut down = vec![pt(0, 1000), pt(3, 0)];
    break_at(&mut down, 1);
    assert_eq!(down[1], pt(1, 667));
}

#[test]
fn insert_space_holds_the_s_value_over_the_gap() {
    let mut pts = ramp();
    insert_space(&mut pts, 128, 192);
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(128, 500),
            pt(192, 500),
            pt(320, 1000),
            pt(576, 3000)
        ]
    );
    // Before the first point everything moves; after the last nothing does.
    let mut pts = vec![pt(64, 1), pt(128, 2)];
    insert_space(&mut pts, 0, 32);
    assert_eq!(pts, [pt(96, 1), pt(160, 2)]);
    insert_space(&mut pts, 500, 600);
    assert_eq!(pts, [pt(96, 1), pt(160, 2)]);
    // A jump at s: the left value holds and the jump moves to s + L.
    let mut pts = vec![pt(0, 0), pt(256, 1000), pt(256, 2000)];
    insert_space(&mut pts, 256, 320);
    assert_eq!(pts, [pt(0, 0), pt(256, 1000), pt(320, 1000), pt(320, 2000)]);
}

#[test]
fn delete_holds_then_jumps_at_e() {
    let mut pts = ramp();
    delete(&mut pts, 128, 384);
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(128, 500),
            pt(384, 500),
            pt(384, 2000),
            pt(512, 3000)
        ]
    );
    // Equal s and e values: no copy.
    let mut pts = vec![pt(0, 100), pt(256, 500), pt(512, 100)];
    delete(&mut pts, 0, 512);
    assert_eq!(pts, [pt(0, 100), pt(512, 100)]);
    // From the first point: it holds, then the jump at e.
    let mut pts = ramp();
    delete(&mut pts, 0, 128);
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(128, 0),
            pt(128, 500),
            pt(256, 1000),
            pt(512, 3000)
        ]
    );
    // From before the first point: no s break, nothing held; the e break starts the curve.
    let mut late = vec![pt(64, 0), pt(192, 1280)];
    delete(&mut late, 0, 128);
    assert_eq!(late, [pt(128, 640), pt(192, 1280)]);
    // Past the last point: the s break holds to the end.
    let mut pts = ramp();
    delete(&mut pts, 384, 600);
    assert_eq!(pts, [pt(0, 0), pt(256, 1000), pt(384, 2000)]);
}

#[test]
fn delete_space_closes_the_gap_with_a_jump() {
    let mut pts = ramp();
    delete_space(&mut pts, 128, 384);
    assert_eq!(pts, [pt(0, 0), pt(128, 500), pt(128, 2000), pt(256, 3000)]);
}

#[test]
fn duplicate_copies_the_range_to_e() {
    let mut pts = ramp();
    duplicate(&mut pts, 0, 256);
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(256, 1000),
            pt(256, 0),
            pt(512, 1000),
            pt(512, 1000),
            pt(768, 3000)
        ]
    );
}

#[test]
fn copy_and_move_to_playhead() {
    // p ≥ e.
    let mut pts = ramp();
    assert!(to_playhead(&mut pts, 0, 256, 512, false));
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(256, 1000),
            pt(512, 3000),
            pt(512, 0),
            pt(768, 1000),
            pt(768, 3000)
        ]
    );
    // p ≤ s: the source moves on by L first.
    let mut pts = ramp();
    assert!(to_playhead(&mut pts, 256, 512, 0, false));
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(0, 1000),
            pt(256, 3000),
            pt(256, 0),
            pt(512, 1000),
            pt(768, 3000)
        ]
    );
    // Move: the source span goes and its gap closes.
    let mut pts = ramp();
    assert!(to_playhead(&mut pts, 256, 512, 0, true));
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(0, 1000),
            pt(256, 3000),
            pt(256, 0),
            pt(512, 1000),
            pt(512, 3000)
        ]
    );
    let mut pts = ramp();
    assert!(to_playhead(&mut pts, 0, 256, 512, true));
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(0, 1000),
            pt(256, 3000),
            pt(256, 0),
            pt(512, 1000),
            pt(512, 3000)
        ]
    );
    // The playhead inside the range (or strictly between s and e): nothing.
    for p in [1, 128, 255] {
        let mut pts = ramp();
        assert!(!to_playhead(&mut pts, 0, 256, p, false));
        assert!(!to_playhead(&mut pts, 0, 256, p, true));
        assert_eq!(pts, ramp());
    }
    // p before the first point has no p break, so nothing is held over the gap: the copy's end value runs
    // into the moved source's start, the rule's literal result.
    let mut pts = vec![pt(128, 0), pt(256, 1000)];
    assert!(to_playhead(&mut pts, 128, 256, 0, false));
    assert_eq!(pts, [pt(0, 0), pt(128, 1000), pt(256, 0), pt(384, 1000)]);
    // p = e is Duplicate; p = s copies in front of the source.
    let (mut a, mut b) = (ramp(), ramp());
    assert!(to_playhead(&mut a, 0, 256, 256, false));
    duplicate(&mut b, 0, 256);
    assert_eq!(a, b);
    let mut pts = ramp();
    assert!(to_playhead(&mut pts, 0, 256, 0, false));
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(0, 0),
            pt(256, 1000),
            pt(256, 0),
            pt(512, 1000),
            pt(768, 3000)
        ]
    );
}

#[test]
fn trim_keeps_only_the_range_from_zero() {
    let mut pts = ramp();
    trim(&mut pts, 128, 384, 512);
    assert_eq!(
        pts,
        [
            pt(0, 0),
            pt(0, 500),
            pt(128, 1000),
            pt(256, 2000),
            pt(256, 3000)
        ]
    );
    // A range from 0 to the loop end changes nothing.
    let mut pts = ramp();
    trim(&mut pts, 0, 512, 512);
    assert_eq!(pts, ramp());
}
