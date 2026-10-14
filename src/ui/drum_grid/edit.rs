//! The free note-grid's pure logic: its time axis and marks, and the edits on a drum clip's hits, each keeping
//! `Clip.events` sorted by position (stable) with the selection mask (one flag per event) in step. Positions are in
//! ticks ([`PPQ`] per beat) as `f64` while editing and whole ticks when stored; lengths are `len_beats`.

use eframe::egui::{Pos2, Rect, Vec2, pos2};

use super::Remembered;
use crate::song::{Clip, Event, PPQ, TimeSig};
use crate::ui::clips::TimeAxis;
use crate::ui::graph_lane::Prop;
use crate::ui::piano_roll::notes::{HANDLE_D, snap_round, sort};
use crate::ui::step_seq::edit as steps;

pub const BEAT: f64 = PPQ as f64;
/// The time zoom k: beat width = U·d/(4k).
pub const K_RANGE: [f32; 2] = [0.01, 1.0];
/// A clip with no saved view opens at this many U per beat.
const NEW_BEAT_U: f32 = 4.0;
/// The drum grid's marks are never closer than this, in U.
pub const MARK_GAP: f32 = 0.6;

/// A beat's width in points at U = `u` points, subdivision `d` (`TDIV` byte 0; 0, which a file may hold, counts as
/// 1) and zoom `k` (clamped).
pub fn beat_w(u: f32, d: u8, k: f32) -> f32 {
    u * f32::from(d.max(1)) / (4.0 * k.clamp(K_RANGE[0], K_RANGE[1]))
}

/// The k of a clip with no saved view: 4 U per beat.
pub fn new_k(d: u8) -> f32 {
    (f32::from(d.max(1)) / (4.0 * NEW_BEAT_U)).clamp(K_RANGE[0], K_RANGE[1])
}

/// The marks between ticks `t0` and `t1` as (tick, line width in device pixels): 4 at a bar, 2 at a beat, 1 finer.
/// Finer marks split the beat by the prime factors of the subdivision d (smallest first, so the step, 1/d beat, is a
/// level), then halve it down to a tick; each level only while its marks are at least `gap` U apart (`u` points;
/// the drum grid's [`MARK_GAP`], the audio editor's 2.4); beats too. Bars are always marked. No width (or NaN)
/// marks nothing.
pub fn marks(t0: f64, t1: f64, beat_w: f32, u: f32, sig: TimeSig, gap: f32) -> Vec<(f64, f32)> {
    // NaN too.
    if beat_w.is_nan() || beat_w <= 0.0 {
        return Vec::new();
    }
    let b = u64::from(sig.bar_beats());
    let (per_beat, every) = mark_level(beat_w, u, sig, gap);
    let unit = BEAT / per_beat as f64;
    let first = ((t0.max(0.0) / unit).ceil() as u64).next_multiple_of(every);
    (first..)
        .step_by(every as usize)
        .map(|i| (i, i as f64 * unit))
        .take_while(|&(_, t)| t <= t1)
        .map(|(i, t)| {
            let w = if i % (per_beat * b) == 0 {
                4.0
            } else if i % per_beat == 0 {
                2.0
            } else {
                1.0
            };
            (t, w)
        })
        .collect()
}

/// The distance in ticks between the visible grid lines.
pub fn mark_spacing(beat_w: f32, u: f32, sig: TimeSig, gap: f32) -> f64 {
    let (per_beat, every) = mark_level(beat_w, u, sig, gap);
    BEAT / per_beat as f64 * every as f64
}

fn mark_level(beat_w: f32, u: f32, sig: TimeSig, gap: f32) -> (u64, u64) {
    // Marks per beat at the finest level far enough apart, or bars only (every b-th beat).
    (mark_levels(sig.steps_per_beat()).into_iter().rev())
        .find(|&k| beat_w / k as f32 >= gap * u)
        .map_or((1, u64::from(sig.bar_beats())), |k| (u64::from(k), 1))
}

/// Marks per beat at each level, coarsest first: 1, then the beat split by `d`'s prime factors (smallest first),
/// then halved, never finer than a tick (a level past [`PPQ`] is dropped).
fn mark_levels(d: u32) -> Vec<u32> {
    let mut levels = vec![1];
    let (mut n, mut p) = (d, 2);
    while n > 1 {
        if n.is_multiple_of(p) {
            levels.push(levels[levels.len() - 1] * p);
            n /= p;
        } else {
            p += 1;
        }
    }
    levels.retain(|&k| k <= PPQ);
    while levels[levels.len() - 1] * 2 <= PPQ {
        levels.push(levels[levels.len() - 1] * 2);
    }
    levels
}

/// A drum hit's (pad, start, length) in ticks.
pub fn hit(e: &Event) -> Option<(u8, f64, f64)> {
    match *e {
        Event::Drum {
            pad,
            pos,
            len_beats,
            ..
        } => Some((pad, f64::from(pos), len_beats * BEAT)),
        _ => None,
    }
}

/// The minimum length in ticks: the snap unit / 8, or 1/128 beat with snap off.
pub fn min_len(snap: Option<f64>) -> f64 {
    snap.map_or(1.0, |g| g / 8.0)
}

/// A tap's new hit in row `pad` at `tick`: the start rounded to the nearest snap unit (whole ticks with snap off), not
/// below 0; the remembered length (at least `min_len`), velocity and pan; pitch centred, no repeat.
pub fn new_hit(pad: u8, tick: f64, snap: Option<f64>, r: &Remembered) -> Event {
    Event::Drum {
        pos: snap_round(tick, snap).max(0.0) as u32,
        len_beats: r.len_beats.max(min_len(snap) / BEAT),
        pad,
        velocity: r.velocity,
        pan: r.pan,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    }
}

/// `Paste` at tick `tick` (as the automation editor's): the clipboard's earliest hit lands on the nearest snap unit
/// (≥ 0), the others keep their offsets from it, their pads and lengths; they join the clip (overlaps allowed) and are
/// the selection.
pub fn paste(
    events: &mut Vec<Event>,
    sel: &mut Vec<bool>,
    hits: &[Event],
    tick: f64,
    snap: Option<f64>,
) {
    let at = snap_round(tick, snap).max(0.0) as u32;
    let first = hits.iter().map(Event::pos).min().unwrap_or(0);
    sel.fill(false);
    for e in hits {
        if let Some((pad, ..)) = hit(e) {
            let i = insert(
                events,
                sel,
                with(*e, at.saturating_add(e.pos() - first), pad, None),
            );
            sel[i] = true;
        }
    }
}

/// Inserts `e` after every event at or before its position, unselected; returns its index.
pub fn insert(events: &mut Vec<Event>, sel: &mut Vec<bool>, e: Event) -> usize {
    let i = events.partition_point(|x| x.pos() <= e.pos());
    events.insert(i, e);
    sel.insert(i, false);
    i
}

/// The `which` hits of `orig` moved: the `anchor`'s start goes to the nearest snap unit of its start + `dt` ticks, the
/// others by as much, none before the clip start; and `dr` rows, clamped to the `rows` rows. Sorted, with `sel`.
#[allow(clippy::too_many_arguments)]
pub fn moved(
    orig: &[Event],
    sel: &[bool],
    which: &[bool],
    anchor: usize,
    dt: f64,
    dr: i32,
    rows: usize,
    snap: Option<f64>,
) -> (Vec<Event>, Vec<bool>) {
    let Some((_, start, _)) = orig.get(anchor).and_then(hit) else {
        return (orig.to_vec(), sel.to_vec());
    };
    let group = || {
        (orig.iter().zip(which))
            .filter(|(_, w)| **w)
            .filter_map(|(e, _)| hit(e))
    };
    let first = group().map(|h| h.1).fold(f64::MAX, f64::min);
    let shift = (snap_round(start + dt, snap) - start).max(-first);
    let pads = || group().map(|h| i32::from(h.0));
    let top = rows.saturating_sub(1) as i32;
    let dr = dr
        .max(-pads().min().unwrap_or(0))
        .min(top - pads().max().unwrap_or(0));
    let mut events: Vec<Event> = (orig.iter().zip(which))
        .map(|(&e, &w)| match hit(&e) {
            Some((pad, pos, _)) if w => {
                let pad = (i32::from(pad) + dr).clamp(0, 255) as u8;
                with(e, (pos + shift).round().max(0.0) as u32, pad, None)
            }
            _ => e,
        })
        .collect();
    let mut sel = sel.to_vec();
    sort(&mut events, &mut sel);
    (events, sel)
}

/// The `which` hits of `orig` resized: the `anchor`'s end goes to the nearest snap unit of its end + `dt` ticks, the
/// others' ends by as much, none shorter than [`min_len`].
pub fn resized(
    orig: &[Event],
    which: &[bool],
    anchor: usize,
    dt: f64,
    snap: Option<f64>,
) -> Vec<Event> {
    let Some((_, start, len)) = orig.get(anchor).and_then(hit) else {
        return orig.to_vec();
    };
    let dl = snap_round(start + len + dt, snap) - (start + len);
    let min = min_len(snap);
    (orig.iter().zip(which))
        .map(|(&e, &w)| match hit(&e) {
            Some((pad, pos, len)) if w => with(e, pos as u32, pad, Some((len + dl).max(min))),
            _ => e,
        })
        .collect()
}

/// The floating `Copy` (the piano roll's): the selected hits again right after them, as many `grid` units on as
/// their span rounds up to (at least one); the copies become the selection.
pub fn copy_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>, grid: f64) {
    let chosen = || {
        (events.iter().zip(sel.iter()))
            .filter(|(_, s)| **s)
            .filter_map(|(e, _)| hit(e))
    };
    let start = chosen().map(|h| h.1).fold(f64::MAX, f64::min);
    let end = chosen().map(|h| h.1 + h.2).fold(f64::MIN, f64::max);
    if start == f64::MAX {
        return;
    }
    let interval = ((end - start) / grid).ceil().max(1.0) * grid;
    let copies: Vec<Event> = (events.iter().zip(sel.iter()))
        .filter(|(_, s)| **s)
        .filter_map(|(&e, _)| hit(&e).map(|h| with(e, (h.1 + interval).round() as u32, h.0, None)))
        .collect();
    sel.fill(false);
    sel.extend(copies.iter().map(|_| true));
    events.extend(copies);
    sort(events, sel);
}

/// The floating `Snap`: the selected hits' starts go to the nearest `grid` line (the piano roll's quantize).
pub fn quantize_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>, grid: f64) {
    for (e, _) in events.iter_mut().zip(sel.iter()).filter(|(_, s)| **s) {
        if let Some((pad, pos, _)) = hit(e) {
            *e = with(*e, snap_round(pos, Some(grid)) as u32, pad, None);
        }
    }
    sort(events, sel);
}

/// The most hits one free-grid Fill adds. Fill has no bound beyond the loop; this only keeps a corrupt loop length
/// from making billions of hits. At d = 8 it is 8192 beats of every-step hits.
// ponytail: a memory guard, not a Fill rule; raise it if a real loop ever needs more.
const FILL_HITS: usize = 1 << 16;

/// `Fill every / each 2 / each 4 step` here: the step mode's fill ([`steps::fill_hits`]) over the whole loop, not the
/// step grid's 512 steps, with hits N/4 beat long whatever d, N = `every`, so hits overlap below d = 4.
pub fn fill(clip: &mut Clip, pad: u8, every: usize, sig: TimeSig) {
    let every = every.max(1);
    steps::fill_hits(clip, pad, every, sig, every as f64 / 4.0, every * FILL_HITS);
}

/// The graph lane's paint at `tick`: every eligible hit of row `pad` (all of them, or only the selected ones while any
/// is selected) whose extent [start, start + length] holds `tick` takes `value` for `prop`. Returns whether one changed.
pub fn paint(
    events: &mut [Event],
    sel: &[bool],
    pad: u8,
    prop: Prop,
    tick: f64,
    value: u16,
) -> bool {
    let any = sel.contains(&true);
    let mut changed = false;
    for (i, e) in events.iter_mut().enumerate() {
        let Some((p, start, len)) = hit(e) else {
            continue;
        };
        let eligible = p == pad && (!any || sel.get(i) == Some(&true));
        if eligible && (start..=start + len).contains(&tick) && prop.get(e) != Some(value) {
            prop.set(e, value);
            changed = true;
        }
    }
    changed
}

/// Where hits sit on screen: the time axis, the top of row 0 and the row pitch, and how many rows there are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub axis: TimeAxis,
    pub row_top: f32,
    pub pitch: f32,
    pub rows: usize,
    /// U, in points.
    pub u: f32,
}

impl Place {
    /// A hit's extent on its row: x from its start to its end, y the row's whole pitch.
    pub fn rect(&self, e: &Event) -> Option<Rect> {
        let (pad, start, len) = hit(e)?;
        let top = self.row_top + f32::from(pad) * self.pitch;
        Some(Rect::from_min_max(
            pos2(self.axis.x(start), top),
            pos2(self.axis.x(start + len), top + self.pitch),
        ))
    }

    /// A hit's resize handle: the piano roll's disc, [`HANDLE_D`] across, just past its right end on its row's
    /// centre line.
    pub fn handle(&self, e: &Event) -> Option<Rect> {
        let r = self.rect(e)?;
        let d = HANDLE_D * self.u;
        Some(Rect::from_center_size(
            pos2(r.right() + d / 2.0, r.center().y),
            Vec2::splat(d),
        ))
    }

    /// The row under `y`, if any.
    pub fn row(&self, y: f32) -> Option<usize> {
        let row = ((y - self.row_top) / self.pitch).floor();
        (row >= 0.0 && (row as usize) < self.rows).then_some(row as usize)
    }

    /// The topmost (last drawn) hit whose extent holds `p` (no minimum width).
    pub fn hit_at(&self, events: &[Event], p: Pos2) -> Option<usize> {
        let row = self.row(p.y)?;
        (0..events.len()).rev().find(|&i| {
            hit(&events[i]).is_some_and(|h| usize::from(h.0) == row)
                && self
                    .rect(&events[i])
                    .is_some_and(|r| r.x_range().contains(p.x))
        })
    }

    /// One flag per event: its extent overlaps `rect`.
    pub fn in_box(&self, events: &[Event], rect: Rect) -> Vec<bool> {
        (events.iter())
            .map(|e| self.rect(e).is_some_and(|r| r.intersects(rect)))
            .collect()
    }
}

/// `e` (a drum hit) at `to` on pad `on`, and `len` ticks long if given; the rest kept.
fn with(e: Event, to: u32, on: u8, len: Option<f64>) -> Event {
    match e {
        Event::Drum {
            len_beats,
            velocity,
            pan,
            value,
            repeat,
            ..
        } => Event::Drum {
            pos: to,
            len_beats: len.map_or(len_beats, |l| l / BEAT),
            pad: on,
            velocity,
            pan,
            value,
            repeat,
        },
        other => other,
    }
}
