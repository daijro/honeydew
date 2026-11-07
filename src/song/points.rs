//! Control points of an automation clip or of one audio-clip envelope, and the editors' pure point math: the
//! gestures and region rows.
//!
//! A point is a positive-key event: `Event::Automation { control: true }`, or `Event::Envelope { control: true }` of
//! one `env_param`. The negative-key steps between them are derived data that [`set_points`] rebuilds (removed and
//! rebuilt, never accumulated). Values are the file's u16 (value = round(f·65535)), so an imported 65535 survives on
//! Every point an edit does not touch.
//!
//! Rows: the value range shows as 127 rows; a value's row is r = (1 − value/max)·126 ([`row`]), 0 at the top, with
//! max = [`PointKey::max`]: 65534 for automation (new editor points top out there) and 65535 for envelopes (rows from
//! value/65535). Ticks are content ticks, [`PPQ`] per beat; a snap unit is in ticks (`None`: Snap off).
//!
//! An automation point also carries the [`Shape`] of the segment that starts at it (Honeydew's own, kept in the clip's
//! `HDSh`, [`super::ClipFileState::shapes`]); envelope points keep the straight line. The ops carry each point's shape
//! with it; a break takes its value on the shaped curve and the segment's shape on both sides.

use super::{Clip, Event, PPQ};
pub use honeydew_audio::automation::{Shape, ShapeKind, shape_value};

/// Which points of a clip: the automation lane's, or an audio clip's envelope `env_param` (0 Level, 1 Pan, 2 Speed;
/// the file key is ±(env_param + 1)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKey {
    Automation,
    Envelope(i16),
}

impl PointKey {
    /// The value at row 0: 65534 for automation, 65535 for envelopes.
    pub fn max(self) -> u16 {
        match self {
            Self::Automation => 65534,
            Self::Envelope(_) => 65535,
        }
    }

    /// `e` is one of this key's events: (pos, value, control).
    fn of(self, e: &Event) -> Option<(u32, f32, bool)> {
        match (self, *e) {
            (
                Self::Automation,
                Event::Automation {
                    pos,
                    value,
                    control,
                },
            ) => Some((pos, value, control)),
            (
                Self::Envelope(k),
                Event::Envelope {
                    pos,
                    env_param,
                    value,
                    control,
                },
            ) if env_param == k => Some((pos, value, control)),
            _ => None,
        }
    }

    fn event(self, pos: u32, value: u16, control: bool) -> Event {
        let value = unit(value);
        match self {
            Self::Automation => Event::Automation {
                pos,
                value,
                control,
            },
            Self::Envelope(env_param) => Event::Envelope {
                pos,
                env_param,
                value,
                control,
            },
        }
    }
}

/// A control point: content tick, stored value and the shape of the segment that starts at it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub tick: u32,
    pub value: u16,
    pub shape: Shape,
}

/// The shape of the automation segment that starts at content tick `tick` of `clip` (the straight line unless its
/// `HDSh` names one).
pub fn shape_at(clip: &Clip, tick: u32) -> Shape {
    let shapes = &clip.file.shapes;
    shapes
        .binary_search_by_key(&tick, |s| s.0)
        .map_or_else(|_| Shape::default(), |i| shapes[i].1)
}

/// The bottom row; rows run 0…126 (127 rows).
pub const LAST_ROW: f64 = 126.0;

/// The stored u16 of a model value (value/65535): round(f·65535), as the file writer stores it.
pub fn raw(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The model value of a stored u16: value/65535.
pub fn unit(value: u16) -> f32 {
    f32::from(value) / 65535.0
}

/// The points of `key` in `clip`, in event order.
pub fn points(clip: &Clip, key: PointKey) -> Vec<Point> {
    (clip.events.iter())
        .filter_map(|e| key.of(e))
        .filter(|&(_, _, control)| control)
        .map(|(tick, value, _)| Point {
            tick,
            value: raw(value),
            shape: match key {
                PointKey::Automation => shape_at(clip, tick),
                PointKey::Envelope(_) => Shape::default(),
            },
        })
        .collect()
}

/// Makes `pts` (sorted by tick, stable) the points of `key` in `clip` and rebuilds that key's steps through
/// `honeydew_audio::automation::AutomationClip`, with their shapes on automation; every other event stays.
pub fn set_points(clip: &mut Clip, key: PointKey, pts: Vec<Point>) {
    use honeydew_audio::automation::{AutomationClip, AutomationPoint};
    let automation = key == PointKey::Automation;
    let pts = pts
        .iter()
        .map(|p| AutomationPoint {
            tick: p.tick.into(),
            value: p.value,
            shape: if automation {
                p.shape
            } else {
                Shape::default()
            },
        })
        .collect();
    // Key 1, a one-tick loop: only the rebuild is used, which sorts the points (stable) and adds the steps.
    let built = AutomationClip::new(0, 1, 1, 0, 1, pts).expect("a positive key and a loop");
    if automation {
        // By tick, the last point's at each: the one whose segment has a length.
        let shapes = &mut clip.file.shapes;
        shapes.clear();
        for p in built.points() {
            let tick = p.tick as u32;
            if shapes.last().is_some_and(|s| s.0 == tick) {
                shapes.pop();
            }
            if p.shape != Shape::default() {
                shapes.push((tick, p.shape));
            }
        }
    }
    clip.events.retain(|e| key.of(e).is_none());
    // A tick is a u32 point's, or a step between two of them.
    (clip.events).extend(
        built
            .events()
            .iter()
            .map(|e| key.event(e.tick as u32, e.value, e.key > 0)),
    );
    clip.events.sort_by_key(Event::pos);
}

/// A value's row, (1 − value/max)·126: 0 at max, 126 at 0 (an imported 65535 sits a little above row 0).
pub fn row(value: u16, max: u16) -> f64 {
    (1.0 - f64::from(value) / f64::from(max)) * LAST_ROW
}

/// The value at row `r`: clamp(trunc(max − r·max/126), 0, max). A millionth absorbs float error: a value's row, moved
/// by whole rows, gives an exact result that is whole or at least 1/42 from whole, which must not truncate one lower.
pub fn value_at_row(r: f64, max: u16) -> u16 {
    let max = f64::from(max);
    (max - r * max / LAST_ROW + 1e-6).trunc().clamp(0.0, max) as u16
}

/// Inserts `p` after every point at or before its tick; returns its index.
pub fn insert(pts: &mut Vec<Point>, p: Point) -> usize {
    let i = pts.partition_point(|q| q.tick <= p.tick);
    pts.insert(i, p);
    i
}

/// Finger tick `t` on the snap grid (`None`: whole ticks), not below 0.
fn snapped(t: f64, snap: Option<f64>) -> u32 {
    snap.map_or(t, |g| (t / g).round() * g).round().max(0.0) as u32
}

/// `pts` sorted by tick (stable), with `sel` in step.
fn sorted(pts: Vec<Point>, sel: &[bool]) -> (Vec<Point>, Vec<bool>) {
    debug_assert_eq!(pts.len(), sel.len());
    let mut both: Vec<(Point, bool)> = pts.into_iter().zip(sel.iter().copied()).collect();
    both.sort_by_key(|(p, _)| p.tick);
    both.into_iter().unzip()
}

/// The selected points' ticks and rows: (first tick, last tick, top row, bottom row).
fn bounds(pts: &[Point], sel: &[bool], max: u16) -> Option<(u32, u32, f64, f64)> {
    let mut chosen = pts.iter().zip(sel).filter(|(_, s)| **s).map(|(p, _)| p);
    let first = chosen.next()?;
    let init = (
        first.tick,
        first.tick,
        row(first.value, max),
        row(first.value, max),
    );
    Some(chosen.fold(init, |(t0, t1, r0, r1), p| {
        let r = row(p.value, max);
        (t0.min(p.tick), t1.max(p.tick), r0.min(r), r1.max(r))
    }))
}

/// Create point: a tap at finger tick `tick` and row `r` = (y − top)·127/V. Tick = the snapped finger tick
/// (rounded), ≥ 0; value = [`value_at_row`]. Returns the points and a selection of just the new one.
pub fn create(
    pts: &[Point],
    tick: f64,
    r: f64,
    snap: Option<f64>,
    max: u16,
) -> (Vec<Point>, Vec<bool>) {
    let mut new = pts.to_vec();
    let p = Point {
        tick: snapped(tick, snap),
        value: value_at_row(r, max),
        ..Point::default()
    };
    let i = insert(&mut new, p);
    let mut sel = vec![false; new.len()];
    sel[i] = true;
    (new, sel)
}

/// Drag: the `sel` points of `orig` moved by the finger's `dt` ticks and `dr` rows. The box's left tick snaps
/// (round) and stays ≥ 0; rows move whole and the box stays within rows 0…126; moved rows commit through
/// [`value_at_row`]. `None` when nothing changed.
///
/// at an edge the box stops on row 0 or 126 exactly (a box between rows still reaches the top value); a
/// point moved in time only keeps its value, so an imported 65535 survives that too.
pub fn moved(
    orig: &[Point],
    sel: &[bool],
    dt: f64,
    dr: f64,
    snap: Option<f64>,
    max: u16,
) -> Option<(Vec<Point>, Vec<bool>)> {
    debug_assert_eq!(orig.len(), sel.len());
    let (left, _, top, bottom) = bounds(orig, sel, max)?;
    let shift = i64::from(snapped(f64::from(left) + dt, snap)) - i64::from(left);
    // An imported 65535 lies above row 0: the box may not move up then, but it need not move down.
    let dr = dr.round().clamp(-top.max(0.0), LAST_ROW - bottom);
    let pts = (orig.iter().zip(sel))
        .map(|(p, s)| match s {
            true => Point {
                tick: (i64::from(p.tick) + shift) as u32,
                value: if dr == 0.0 {
                    p.value
                } else {
                    value_at_row(row(p.value, max) + dr, max)
                },
                ..*p
            },
            false => *p,
        })
        .collect();
    let (pts, sel) = sorted(pts, sel);
    (pts != orig).then_some((pts, sel))
}

/// Right-edge handle: with ≥ 2 selected points, the box's right edge moves by the finger's `dt` ticks and snaps
/// (never left of the box's left edge), and the selected ticks scale about the left edge. `None` when nothing
/// changed.
pub fn scaled(
    orig: &[Point],
    sel: &[bool],
    dt: f64,
    snap: Option<f64>,
) -> Option<(Vec<Point>, Vec<bool>)> {
    let (_, right, _, _) = bounds(orig, sel, 1)?;
    scaled_to(orig, sel, |left| {
        f64::from(snapped(f64::from(right) + dt, snap)).max(left)
    })
}

/// The ↔ scale: with ≥ 2 selected points, the box's right edge goes to `to(left edge)` (ticks, at least the left
/// edge) and the selected ticks scale about the left edge. `None` when nothing changed or the box has no width.
pub fn scaled_to(
    orig: &[Point],
    sel: &[bool],
    to: impl FnOnce(f64) -> f64,
) -> Option<(Vec<Point>, Vec<bool>)> {
    debug_assert_eq!(orig.len(), sel.len());
    if sel.iter().filter(|s| **s).count() < 2 {
        return None;
    }
    let (left, right, _, _) = bounds(orig, sel, 1)?;
    let width = f64::from(right - left);
    if width == 0.0 {
        return None;
    }
    let k = (to(f64::from(left)) - f64::from(left)).max(0.0) / width;
    let pts = (orig.iter().zip(sel))
        .map(|(p, s)| match s {
            true => Point {
                tick: left + (f64::from(p.tick - left) * k).round() as u32,
                ..*p
            },
            false => *p,
        })
        .collect();
    let (pts, sel) = sorted(pts, sel);
    (pts != orig).then_some((pts, sel))
}

/// Marquee: the points whose tick lies in `ticks` and whose one-row band [r, r + 1] overlaps `rows` (both
/// inclusive, in either order).
pub fn marquee(pts: &[Point], ticks: [f64; 2], rows: [f64; 2], max: u16) -> Vec<bool> {
    let (t0, t1) = (ticks[0].min(ticks[1]), ticks[0].max(ticks[1]));
    let (r0, r1) = (rows[0].min(rows[1]), rows[0].max(rows[1]));
    pts.iter()
        .map(|p| {
            let r = row(p.value, max);
            (t0..=t1).contains(&f64::from(p.tick)) && r <= r1 && r + 1.0 >= r0
        })
        .collect()
}

/// A copied point: (beat, value/max, shape).
pub type Copied = (f64, f64, Shape);

/// `Copy`: each selected point's (beat, value/max, shape).
pub fn copy(pts: &[Point], sel: &[bool], max: u16) -> Vec<Copied> {
    (pts.iter().zip(sel))
        .filter(|(_, s)| **s)
        .map(|(p, _)| {
            (
                f64::from(p.tick) / f64::from(PPQ),
                f64::from(p.value) / f64::from(max),
                p.shape,
            )
        })
        .collect()
}

/// Paste at finger tick `tick`: the earliest copied point lands on the snapped tick (≥ 0), the others keep their
/// offsets; values are the copied ones (×max, so full scale stays full scale across keys), and so are the shapes.
/// Returns the points and a selection of just the pasted ones.
pub fn paste(
    pts: &[Point],
    clipboard: &[Copied],
    tick: f64,
    snap: Option<f64>,
    max: u16,
) -> (Vec<Point>, Vec<bool>) {
    let at = f64::from(snapped(tick, snap));
    let first = clipboard.iter().map(|c| c.0).fold(f64::INFINITY, f64::min);
    let (mut new, mut sel) = (pts.to_vec(), vec![false; pts.len()]);
    for &(beat, f, shape) in clipboard {
        let p = Point {
            tick: (at + (beat - first) * f64::from(PPQ)).round() as u32,
            value: (f * f64::from(max)).round().clamp(0.0, 65535.0) as u16,
            shape,
        };
        let i = insert(&mut new, p);
        sel.insert(i, true);
    }
    (new, sel)
}

/// The index of the first point at `t`.
fn first_at(pts: &[Point], t: u32) -> Option<usize> {
    let i = pts.partition_point(|p| p.tick < t);
    (pts.get(i)?.tick == t).then_some(i)
}

/// The point a break at `t` adds: strictly between two points' ticks, the value on the curve between the last point
/// before `t` and the first after (rounded), with that segment's shape. `None` where a point is at `t` or there is no
/// curve.
fn between(pts: &[Point], t: u32) -> Option<Point> {
    let i = pts.partition_point(|p| p.tick < t);
    let (a, b) = (*pts.get(i.checked_sub(1)?)?, *pts.get(i)?);
    let f = f64::from(t - a.tick) / f64::from(b.tick - a.tick);
    let (va, vb) = (f64::from(a.value), f64::from(b.value));
    (b.tick > t).then(|| Point {
        tick: t,
        value: shape_value(a.shape, va, vb, f).round() as u16,
        shape: a.shape,
    })
}

/// Breaks at every tick of `ts`, each on the curve as it was before any of them.
fn breaks(pts: &mut Vec<Point>, ts: &[u32]) {
    let new: Vec<Point> = ts.iter().filter_map(|&t| between(pts, t)).collect();
    for p in new {
        if first_at(pts, p.tick).is_none() {
            insert(pts, p);
        }
    }
}

/// Break at t: the first point at `t`, or a new one there with the value interpolated between its neighbours.
/// `None` before the first point or after the last.
pub fn break_at(pts: &mut Vec<Point>, t: u32) -> Option<usize> {
    breaks(pts, &[t]);
    first_at(pts, t)
}

/// Opens `len` ticks at `at`: the points after the break at `at` move on by `len` and the break's value is re-added
/// at at + len, so it holds over the gap. Without a break, the points from `at` on move (none after the last point).
fn open_gap(pts: &mut Vec<Point>, at: u32, len: u32) {
    let from = match break_at(pts, at) {
        Some(i) => {
            let held = Point {
                tick: at.saturating_add(len),
                ..pts[i]
            };
            pts.insert(i + 1, held);
            i + 2
        }
        None => pts.partition_point(|p| p.tick < at),
    };
    for p in &mut pts[from..] {
        p.tick = p.tick.saturating_add(len);
    }
}

/// `Delete`'s removal: breaks at s and e, then the points in [s, e) go but the s break. Returns the s break.
fn remove(pts: &mut Vec<Point>, s: u32, e: u32) -> Option<Point> {
    breaks(pts, &[s, e]);
    let keep = first_at(pts, s);
    let mut i = 0;
    pts.retain(|p| {
        let kept = Some(i) == keep || !(s..e).contains(&p.tick);
        i += 1;
        kept
    });
    keep.map(|i| pts[i])
}

/// `Insert space` over [s, e].
pub fn insert_space(pts: &mut Vec<Point>, s: u32, e: u32) {
    open_gap(pts, s, e - s);
}

/// `Delete` over [s, e] (also an envelope's `Clear`): the s value holds to e, where the curve jumps on.
pub fn delete(pts: &mut Vec<Point>, s: u32, e: u32) {
    if let Some(held) = remove(pts, s, e)
        && let Some(j) = first_at(pts, e)
        && pts[j].value != held.value
    {
        pts.insert(j, Point { tick: e, ..held });
    }
}

/// `Delete space` over [s, e]: `Delete`'s removal, then the later points close the gap (a jump at s).
pub fn delete_space(pts: &mut Vec<Point>, s: u32, e: u32) {
    remove(pts, s, e);
    for p in pts.iter_mut().filter(|p| p.tick > s) {
        p.tick -= e - s;
    }
}

/// `Duplicate`: copy [s, e] to e.
pub fn duplicate(pts: &mut Vec<Point>, s: u32, e: u32) {
    to_playhead(pts, s, e, e, false);
}

/// `Copy to playhead` (`mv` false) / `Move to playhead` (`mv` true) with the playhead at `p`. Does nothing and
/// returns false unless p ≤ s or p ≥ e.
///
/// The s…e points are those from the first at s through the first at e (a jump at e belongs after the range); the
/// copies go right after the p break. With p before the first point there is no p break, so nothing is held
/// over the gap and the copy's last value runs straight into the moved source.
pub fn to_playhead(pts: &mut Vec<Point>, s: u32, e: u32, p: u32, mv: bool) -> bool {
    if s < p && p < e {
        return false;
    }
    let len = e - s;
    breaks(pts, &[p, s, e]);
    let from = pts.partition_point(|q| q.tick < s);
    let to = pts.partition_point(|q| q.tick < e) + usize::from(first_at(pts, e).is_some());
    let copies: Vec<Point> = (pts[from..to].iter())
        .map(|q| Point {
            tick: (q.tick - s).saturating_add(p),
            ..*q
        })
        .collect();
    open_gap(pts, p, len);
    let at = first_at(pts, p).map_or_else(|| pts.partition_point(|q| q.tick < p), |i| i + 1);
    pts.splice(at..at, copies);
    if mv {
        // The source moved on by L if it lay after p.
        let from = if p <= s { s + len } else { s };
        delete_space(pts, from, from + len);
    }
    true
}

/// `Trim selection`: `Delete space` over [e, loop end], then over [0, s].
pub fn trim(pts: &mut Vec<Point>, s: u32, e: u32, loop_end: u32) {
    if loop_end > e {
        delete_space(pts, e, loop_end);
    }
    if s > 0 {
        delete_space(pts, 0, s);
    }
}

#[cfg(test)]
mod tests;
