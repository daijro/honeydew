//! The drum-lane edits behind the step grid and the row menu, on song data only.
//! A drum hit is an [`Event::Drum`] whose `pad` is the row.

use std::collections::{BTreeMap, HashMap};

use honeydew_audio::sequencer::apply_drum_swing;

use crate::song::{Clip, Event, Lane, LaneKind, PPQ, TimeSig};
use crate::ui::graph_lane::Prop;

// The grid has d = `TDIV` byte 0 steps a beat; step i starts at tick round(i·PPQ/d) ([`TimeSig::step_start`]). A new
// pattern's loop is one bar (b = byte 1 beats).

/// The most steps Fill writes and a copy or paste reaches: 32 bars of 4/4 (against a file's huge loop).
pub const MAX_STEPS: usize = 512;

/// Rows of the grid: one per kit pad, and more if events use higher pads.
pub fn row_count(lane: &Lane, kit_pads: usize) -> usize {
    lane.clips
        .iter()
        .flat_map(|c| &c.events)
        .filter_map(pad_of)
        .map(|p| usize::from(p) + 1)
        .max()
        .unwrap_or(0)
        .max(kit_pads)
}

fn pad_of(e: &Event) -> Option<u8> {
    match *e {
        Event::Drum { pad, .. } => Some(pad),
        _ => None,
    }
}

fn with_pad(mut e: Event, to: u8) -> Event {
    if let Event::Drum { pad, .. } = &mut e {
        *pad = to;
    }
    e
}

/// The step of a hit at tick `pos`: llround(start·d) (the active step and Rotate). A hit Swing moved past half a step
/// (at d = 8, Swing over 0.5) is on the next one.
pub fn step_of(pos: u32, sig: TimeSig) -> usize {
    // `as` saturates.
    (f64::from(pos) * f64::from(sig.steps_per_beat()) / f64::from(PPQ)).round() as usize
}

/// The ticks whose step ([`step_of`]) is `step`, first and last.
fn step_span(step: usize, sig: TimeSig) -> (i64, i64) {
    let w = f64::from(PPQ) / f64::from(sig.steps_per_beat());
    let first = ((step as f64 - 0.5) * w).ceil().max(0.0) as i64;
    (first, ((step as f64 + 0.5) * w).ceil() as i64 - 1)
}

fn in_step(e: &Event, pad: u8, step: usize, sig: TimeSig) -> bool {
    cell_of(e, sig) == Some((pad, step))
}

/// A hit at the start of `step`, pitch centred, no repeat.
fn hit(pad: u8, step: usize, len_beats: f64, velocity: u8, pan: u8, sig: TimeSig) -> Event {
    Event::Drum {
        pos: sig.step_start(step),
        len_beats,
        pad,
        velocity,
        pan,
        value: Event::DEFAULT_PITCH,
        repeat: 0,
    }
}

/// Inserts `e` after every event at or before its position, keeping the clip sorted.
fn insert(events: &mut Vec<Event>, e: Event) {
    let i = events.partition_point(|x| x.pos() <= e.pos());
    events.insert(i, e);
}

/// The hit each (pad, step) cell shows, on any step (the grid has no end): of several in a cell, the last in clip
/// order (drawn on top, as the graph lane draws them).
pub fn cells(clip: &Clip, sig: TimeSig) -> HashMap<(u8, usize), &Event> {
    (clip.events.iter())
        .filter_map(|e| Some((cell_of(e, sig)?, e)))
        .collect()
}

/// A step's start tick `tick` moved by the drum Swing `amount` as the Swing dialog moves it (odd steps start
/// 0.125·amount beat late); unmoved for an invalid amount.
fn swung(tick: u32, sig: TimeSig, amount: f64) -> u32 {
    let mut t = [i64::from(tick)];
    match apply_drum_swing(&mut t, sig.steps_per_beat(), amount, false) {
        Ok(()) => u32::try_from(t[0]).unwrap_or(tick),
        Err(_) => tick,
    }
}

/// Removes every hit of `pad` inside `step`, or, when there is none, adds one a step long with `velocity` and `pan`
/// (the remembered values), at the step's start moved by the lane's Swing: `swing` is (the song's `TDIV`, amount),
/// applied as the Swing dialog does ([`swing`]; odd subdivisions only). Returns the cell of the hit it added, if it
/// added one ([`cell_of`]: the next step for a hit Swing moved past half a step).
pub fn toggle(
    clip: &mut Clip,
    pad: u8,
    step: usize,
    velocity: u8,
    pan: u8,
    (sig, amount): (TimeSig, f64),
) -> Option<(u8, usize)> {
    let before = clip.events.len();
    clip.events.retain(|e| !in_step(e, pad, step, sig));
    if clip.events.len() != before {
        return None;
    }
    let len = 1.0 / f64::from(sig.steps_per_beat());
    let e = hit(pad, step, len, velocity, pan, sig);
    let e = with_pos(e, swung(e.pos(), sig, amount));
    insert(&mut clip.events, e);
    cell_of(&e, sig)
}

/// `Clear`: removes the row's hits.
pub fn clear(clip: &mut Clip, pad: u8) {
    clip.events.retain(|e| pad_of(e) != Some(pad));
}

/// `Fill every step` (1), `Fill each 2 step` (2), `Fill each 4 step` (4) in step mode: [`fill_hits`] with hits 0
/// long, on the grid's at most [`MAX_STEPS`] steps.
pub fn fill(clip: &mut Clip, pad: u8, every: usize, sig: TimeSig) {
    fill_hits(clip, pad, every, sig, 0.0, MAX_STEPS);
}

/// Fill, N = `every`, both drum modes: every hit of row `pad` goes, past the loop too, then for j = 0, N, 2N, … below
/// llround(loopLen·d), d = TDIV byte 0, a hit at step j's start (j/d beat, [`TimeSig::step_start`]), `len_beats` long,
/// at velocity 255 and pan centred (the remembered values are for taps). Over at most `most` subdivisions (a NaN or
/// negative loop makes none).
pub fn fill_hits(
    clip: &mut Clip,
    pad: u8,
    every: usize,
    sig: TimeSig,
    len_beats: f64,
    most: usize,
) {
    clear(clip, pad);
    // `as` saturates: NaN and negatives give 0.
    let count =
        ((clip.loop_len_beats * f64::from(sig.steps_per_beat())).round() as usize).min(most);
    for j in (0..count).step_by(every.max(1)) {
        (clip.events).push(hit(pad, j, len_beats, 255, Event::DEFAULT_PAN, sig));
    }
    // Stable: the new hits follow what was at their positions, as `insert` puts them, in one sort.
    clip.events.sort_by_key(Event::pos);
}

/// `Rotate right` / `Rotate left`: over n = llround(L·d) steps, every hit of row `pad` goes, and each occupied step i
/// (a hit's step is llround(start·d)) comes back at the start of step (i ± 1) mod n, keeping its length and values;
/// several on one step collapse to the last in clip order. The lane's Swing (`swing` is the song's `TDIV` and the
/// amount) is re-applied to the moved hits at once, so a hit moved onto the loop's last (odd) step ends past the loop,
/// which the host's refit then doubles. A loop under half a step rotates nothing.
pub fn rotate(clip: &mut Clip, pad: u8, right: bool, (sig, amount): (TimeSig, f64)) {
    let d = f64::from(sig.steps_per_beat());
    let n = (clip.loop_len_beats * d).round();
    // NaN too (`as` saturates a huge loop).
    if n.is_nan() || n < 1.0 {
        return;
    }
    let (n, r) = (n as i64, if right { 1 } else { -1 });
    let mut moved = BTreeMap::new();
    clip.events.retain(|e| {
        if pad_of(e) != Some(pad) {
            return true;
        }
        let i = step_of(e.pos(), sig) as i64;
        moved.insert((i + r).rem_euclid(n), *e);
        false
    });
    for (j, e) in moved {
        let start = sig.step_start(usize::try_from(j).unwrap_or(0));
        insert(&mut clip.events, with_pos(e, swung(start, sig, amount)));
    }
}

/// `pos` in step `from` moved to step `to`, keeping its offset from the step's start (before it, for a hit nearest
/// the step from below) but never out of `to`'s ticks ([`step_span`]; steps of a fractional width differ by a tick).
fn to_step(pos: u32, from: usize, to: usize, sig: TimeSig) -> u32 {
    let off = i64::from(pos) - i64::from(sig.step_start(from));
    let (first, last) = step_span(to, sig);
    let at = (i64::from(sig.step_start(to)) + off).clamp(first, last);
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// The graph lane's paint: `pad`'s hits in `step` take `value` for `prop`.
pub fn set_prop(clip: &mut Clip, pad: u8, step: usize, prop: Prop, value: u16, sig: TimeSig) {
    for e in &mut clip.events {
        if cell_of(e, sig) == Some((pad, step)) {
            prop.set(e, value);
        }
    }
}

/// The (pad, step) cell of a drum hit ([`step_of`]).
pub fn cell_of(e: &Event, sig: TimeSig) -> Option<(u8, usize)> {
    pad_of(e).map(|pad| (pad, step_of(e.pos(), sig)))
}

/// The floating `Copy`: the hits in `cells` are copied as many steps on as the cells span, first to last (clip
/// `Copy` on a one-step grid), each copy replacing what its cell held; returns the copies' cells. Copies past
/// [`MAX_STEPS`] are dropped; each keeps its offset into its step. (The host fits the loop to the hits after it, as
/// after every hit edit.)
pub fn copy_steps(clip: &mut Clip, cells: &[(u8, usize)], sig: TimeSig) -> Vec<(u8, usize)> {
    let steps = cells.iter().map(|c| c.1);
    let (Some(first), Some(last)) = (steps.clone().min(), steps.max()) else {
        return Vec::new();
    };
    let shift = last - first + 1;
    let copies: Vec<Event> = (clip.events.iter())
        .filter_map(|&e| Some((e, cell_of(&e, sig)?)))
        // ponytail: copies past MAX_STEPS are dropped, a 32-bar ceiling; lift it when the grid's loop may outgrow it.
        .filter(|(_, c)| cells.contains(c) && c.1 + shift < MAX_STEPS)
        .map(|(e, c)| with_pos(e, to_step(e.pos(), c.1, c.1 + shift, sig)))
        .collect();
    land(clip, copies, sig)
}

/// `Paste` at step `at` (as the automation editor's): the first step of the clipboard's `hits` lands on `at`, the
/// others keep their pads and their distances in steps from it; each keeps its offset into its step, replaces what its
/// cell held, and is dropped past [`MAX_STEPS`]. Returns the pasted cells.
pub fn paste_steps(clip: &mut Clip, hits: &[Event], at: usize, sig: TimeSig) -> Vec<(u8, usize)> {
    let cells = hits.iter().filter_map(|e| Some((*e, cell_of(e, sig)?)));
    let Some(first) = cells.clone().map(|(_, c)| c.1).min() else {
        return Vec::new();
    };
    let pasted = cells
        .map(|(e, c)| (e, c.1, at + (c.1 - first)))
        // ponytail: as copy_steps, pasted hits past MAX_STEPS are dropped.
        .filter(|&(_, _, to)| to < MAX_STEPS)
        .map(|(e, from, to)| with_pos(e, to_step(e.pos(), from, to, sig)))
        .collect();
    land(clip, pasted, sig)
}

/// Puts `hits` into the clip, each replacing what its cell held; returns their cells, sorted.
fn land(clip: &mut Clip, hits: Vec<Event>, sig: TimeSig) -> Vec<(u8, usize)> {
    let mut targets: Vec<(u8, usize)> = hits.iter().filter_map(|e| cell_of(e, sig)).collect();
    targets.sort_unstable();
    targets.dedup();
    clip.events
        .retain(|e| cell_of(e, sig).is_none_or(|c| !targets.contains(&c)));
    for e in hits {
        insert(&mut clip.events, e);
    }
    targets
}

/// The floating `Delete`: removes every hit in `cells`.
pub fn delete_steps(clip: &mut Clip, cells: &[(u8, usize)], sig: TimeSig) {
    clip.events
        .retain(|e| cell_of(e, sig).is_none_or(|c| !cells.contains(&c)));
}

fn with_pos(mut e: Event, to: u32) -> Event {
    if let Event::Drum { pos, .. } = &mut e {
        *pos = to;
    }
    e
}

/// The event lists of every clip in the channel's drum lanes: pad indices are the channel's.
fn drum_events(lanes: &mut [Lane]) -> impl Iterator<Item = &mut Vec<Event>> {
    lanes
        .iter_mut()
        .filter(|l| l.kind == LaneKind::Drums)
        .flat_map(|l| l.clips.iter_mut().map(|c| &mut c.events))
}

/// `Clone`: in every drum clip of the channel, a copy of `pad`'s hits becomes row `pad + 1` and the later rows
/// move down one. The menu disables `Clone` once the rows fill the 22 pads; past pad 255 (pads are `u8`) nothing
/// moves.
pub fn clone_row(lanes: &mut [Lane], pad: u8) {
    let Some(below) = pad.checked_add(1) else {
        return;
    };
    for events in drum_events(lanes) {
        let copies: Vec<Event> = events
            .iter()
            .filter(|e| pad_of(e) == Some(pad))
            .map(|e| with_pad(*e, below))
            .collect();
        for e in events.iter_mut() {
            if let Some(p) = pad_of(e)
                && p > pad
            {
                *e = with_pad(*e, p.saturating_add(1));
            }
        }
        for e in copies {
            insert(events, e);
        }
    }
}

/// `Delete`: removes `pad`'s hits from every drum clip of the channel; the later rows move up one.
pub fn delete_row(lanes: &mut [Lane], pad: u8) {
    for events in drum_events(lanes) {
        events.retain(|e| pad_of(e) != Some(pad));
        for e in events.iter_mut() {
            if let Some(p) = pad_of(e)
                && p > pad
            {
                *e = with_pad(*e, p - 1);
            }
        }
    }
}

/// The drum editor's Swing (regular grid): every hit moves to the audio crate's `apply_drum_swing` of its position in
/// `from`, the clip's positions (one per event) before the swing edit. Playback plays the moved hits as they are; d is
/// the subdivision (`TDIV` byte 0; 0 counts as 1). Nothing happens when `from` no longer matches the clip or the amount
/// is invalid.
pub fn swing(clip: &mut Clip, from: &[u32], sig: TimeSig, amount: f64) {
    if from.len() != clip.events.len() {
        return;
    }
    let mut ticks: Vec<i64> = from.iter().map(|&p| i64::from(p)).collect();
    if apply_drum_swing(&mut ticks, sig.steps_per_beat(), amount, false).is_err() {
        return;
    }
    for (e, tick) in clip.events.iter_mut().zip(ticks) {
        if let Event::Drum { pos, .. } = e {
            *pos = u32::try_from(tick).unwrap_or(0);
        }
    }
    clip.events.sort_by_key(Event::pos);
}
