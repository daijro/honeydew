//! The piano roll's pure logic: snap modes and grid math, the view's geometry and hit testing, edits on a clip's
//! events with a selection mask kept in step, and the song mappings (playhead, ghost notes).
//!
//! Positions are in ticks ([`PPQ`] per beat) as `f64` while editing and are rounded to whole ticks when stored.
//! Every edit keeps `Clip.events` sorted by position (stable), and the selection mask (one `bool` per event)
//! moves with its event.

use std::cmp::Reverse;

use eframe::egui::{Pos2, Rect, pos2};

use crate::song::scales::{CHROMATIC, SCALE_MASKS, SCALES, scale_from_stored};
use crate::song::{Clip, Event, LaneKind, PPQ, Song, TimeSig};
use crate::ui::keyboard_settings::KEYS;

const BEAT: f64 = PPQ as f64;
/// The highest MIDI key.
pub const TOP_KEY: u8 = 127;
/// The smallest cell Auto snaps to, in U.
const MIN_CELL: f32 = 0.4;
/// How far, in rows, a touch-down above or below a note's row still binds it.
const ROW_BAND: f32 = 0.2;

/// The snap menu, in menu order; the discriminant is the stored snap id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapMode {
    Auto,
    SixthStep,
    QuarterStep,
    ThirdStep,
    HalfStep,
    Step,
    SixthBeat,
    QuarterBeat,
    ThirdBeat,
    HalfBeat,
    Beat,
    Bar,
}

impl SnapMode {
    pub const ALL: [Self; 12] = [
        Self::Auto,
        Self::SixthStep,
        Self::QuarterStep,
        Self::ThirdStep,
        Self::HalfStep,
        Self::Step,
        Self::SixthBeat,
        Self::QuarterBeat,
        Self::ThirdBeat,
        Self::HalfBeat,
        Self::Beat,
        Self::Bar,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::SixthStep => "1/6 Step",
            Self::QuarterStep => "1/4 Step",
            Self::ThirdStep => "1/3 Step",
            Self::HalfStep => "1/2 Step",
            Self::Step => "Step",
            Self::SixthBeat => "1/6 Beat",
            Self::QuarterBeat => "1/4 Beat",
            Self::ThirdBeat => "1/3 Beat",
            Self::HalfBeat => "1/2 Beat",
            Self::Beat => "Beat",
            Self::Bar => "Bar",
        }
    }

    /// The grid in ticks (a step is 1/d beat, a bar b beats), `None` for Auto, which follows the zoom
    /// ([`auto_grid`]).
    pub fn ticks(self, sig: TimeSig) -> Option<f64> {
        let step = sig.step_ticks();
        Some(match self {
            Self::Auto => return None,
            Self::SixthStep => step / 6.0,
            Self::QuarterStep => step / 4.0,
            Self::ThirdStep => step / 3.0,
            Self::HalfStep => step / 2.0,
            Self::Step => step,
            Self::SixthBeat => BEAT / 6.0,
            Self::QuarterBeat => BEAT / 4.0,
            Self::ThirdBeat => BEAT / 3.0,
            Self::HalfBeat => BEAT / 2.0,
            Self::Beat => BEAT,
            Self::Bar => BEAT * f64::from(sig.bar_beats()),
        })
    }
}

/// The snap button's state: a tap toggles `on`, its long-press menu picks `mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snap {
    pub on: bool,
    pub mode: SnapMode,
}

impl Default for Snap {
    fn default() -> Self {
        Self {
            on: true,
            mode: SnapMode::Auto,
        }
    }
}

impl Snap {
    /// The mode's grid in ticks, with `auto` (from [`auto_grid`]) standing in for Auto.
    pub fn grid(self, auto: f64, sig: TimeSig) -> f64 {
        self.mode.ticks(sig).unwrap_or(auto)
    }

    /// What positions snap to: the grid while on, nothing while off.
    pub fn active(self, auto: f64, sig: TimeSig) -> Option<f64> {
        self.on.then(|| self.grid(auto, sig))
    }
}

/// Auto's grid: the finest of step, beat and bar that is at least [`MIN_CELL`] wide at `beat_w` points a beat.
pub fn auto_grid(beat_w: f32, u: f32, sig: TimeSig) -> f64 {
    let min = MIN_CELL * u;
    if beat_w / sig.steps_per_beat() as f32 >= min {
        sig.step_ticks()
    } else if beat_w >= min {
        BEAT
    } else {
        BEAT * f64::from(sig.bar_beats())
    }
}

/// The start of the `grid` cell holding `t` (whole ticks when `None`): a tap places a note in the cell it hits. The
/// nudge keeps a tap right on a line in that line's cell.
pub fn snap_floor(t: f64, grid: Option<f64>) -> f64 {
    match grid {
        Some(g) => ((t / g + 1e-6).floor() * g).round(),
        None => t.floor(),
    }
}

/// `t` rounded to the nearest line of the `grid` (whole ticks when `None`).
pub fn snap_round(t: f64, grid: Option<f64>) -> f64 {
    match grid {
        Some(g) => ((t / g).round() * g).round(),
        None => t.round(),
    }
}

/// A clip editor's selection handle (the end capsule, the ↔ bar) dragged to tick `finger`: the selection box's new
/// end, `finger` rounded to the `grid` (whole ticks when `None`), at least grid/8 (1/128 beat without snap) after the
/// box's start `start`. Only the box's end snaps, never each item's.
pub fn box_end(finger: f64, start: f64, grid: Option<f64>) -> f64 {
    snap_round(finger, grid).max(start + grid.map_or(1.0, |g| g / 8.0))
}

/// `(pos, len, key)` of a note event, in ticks; `None` for other events and for key bytes above 127, which a file
/// may hold but the roll neither draws nor edits.
pub fn note(e: &Event) -> Option<(f64, f64, u8)> {
    match *e {
        Event::Note {
            pos,
            len_beats,
            key,
            ..
        } if key <= TOP_KEY => Some((f64::from(pos), len_beats * BEAT, key)),
        _ => None,
    }
}

fn note_event(pos: f64, len: f64, key: u8) -> Event {
    let blank = Event::Note {
        pos: 0,
        len_beats: 0.0,
        key: 0,
        velocity: Event::DEFAULT_VELOCITY,
        pan: Event::DEFAULT_PAN,
        slide: false,
    };
    with(blank, pos, len, key)
}

/// `e` (a note) with a new position, length or key; velocity, pan and the slide flag are kept.
fn with(e: Event, pos: f64, len: f64, key: u8) -> Event {
    match e {
        Event::Note {
            velocity,
            pan,
            slide,
            ..
        } => Event::Note {
            pos: pos.round().max(0.0) as u32,
            len_beats: len / BEAT,
            key,
            velocity,
            pan,
            slide,
        },
        other => other,
    }
}

/// Where the content sits on screen: `origin` is tick 0's x and the top of key 127's row; cells in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geom {
    pub origin: Pos2,
    pub beat_w: f32,
    pub row_h: f32,
}

impl Geom {
    pub fn x(&self, tick: f64) -> f32 {
        self.origin.x + (tick / BEAT) as f32 * self.beat_w
    }

    pub fn tick(&self, x: f32) -> f64 {
        f64::from((x - self.origin.x) / self.beat_w) * BEAT
    }

    /// The top of `key`'s row.
    pub fn y(&self, key: u8) -> f32 {
        self.origin.y + f32::from(TOP_KEY - key) * self.row_h
    }

    /// The key whose row contains `y`.
    pub fn key(&self, y: f32) -> Option<u8> {
        let row = ((y - self.origin.y) / self.row_h).floor();
        (0.0..=f32::from(TOP_KEY))
            .contains(&row)
            .then(|| TOP_KEY - row as u8)
    }

    /// The full cell span of a note event.
    pub fn note_rect(&self, e: &Event) -> Option<Rect> {
        let (pos, len, key) = note(e)?;
        let top = self.y(key);
        Some(Rect::from_min_max(
            pos2(self.x(pos), top),
            pos2(self.x(pos + len), top + self.row_h),
        ))
    }
}

/// What the pointer is over (the selection's handles aside, which the roll checks first).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Body(usize),
    Empty { tick: f64, key: u8 },
}

/// The note [`note_at`] binds at `p`, or the empty cell there. `None` left of tick 0 or outside the key range.
pub fn hit_test(g: &Geom, events: &[Event], p: Pos2) -> Option<Hit> {
    if let Some(i) = note_at(g, events, p) {
        return Some(Hit::Body(i));
    }
    let tick = g.tick(p.x);
    let key = g.key(p.y)?;
    (tick >= 0.0).then_some(Hit::Empty { tick, key })
}

/// `p` in rows from the top of key 127's row, and in ticks.
fn row_tick(g: &Geom, p: Pos2) -> (f32, f64) {
    ((p.y - g.origin.y) / g.row_h, g.tick(p.x))
}

/// The note a touch-down at `p` binds: one whose [start, start + length) holds `p` in time and whose key row, widened
/// by [`ROW_BAND`] rows above and below, holds it; among several, the nearest by vertical distance outside its row (0
/// inside), the topmost (last drawn) on a tie.
pub fn note_at(g: &Geom, events: &[Event], p: Pos2) -> Option<usize> {
    let (row, tick) = row_tick(g, p);
    (events.iter().enumerate().rev())
        .filter_map(|(i, e)| {
            let (pos, len, key) = note(e)?;
            let top = f32::from(TOP_KEY - key);
            let band = top - ROW_BAND..top + 1.0 + ROW_BAND;
            let off = (top - row).max(row - (top + 1.0)).max(0.0);
            ((pos..pos + len).contains(&tick) && band.contains(&row)).then_some((off, i))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i)| i)
}

/// `p` lies on note `e` itself, its exact row [127 − key, 128 − key) and [start, start + length): a tap's re-check.
pub fn on_note(g: &Geom, e: &Event, p: Pos2) -> bool {
    let (row, tick) = row_tick(g, p);
    note(e).is_some_and(|(pos, len, key)| {
        let top = f32::from(TOP_KEY - key);
        (pos..pos + len).contains(&tick) && (top..top + 1.0).contains(&row)
    })
}

/// The selection box of the flagged notes: their earliest start to their latest end, the highest one's row top to the
/// lowest one's row bottom.
pub fn sel_box(g: &Geom, events: &[Event], which: &[bool]) -> Option<Rect> {
    (events.iter().zip(which))
        .filter(|(_, s)| **s)
        .filter_map(|(e, _)| g.note_rect(e))
        .reduce(Rect::union)
}

/// One flag per event: its note touches `rect`.
pub fn in_box(g: &Geom, events: &[Event], rect: Rect) -> Vec<bool> {
    events
        .iter()
        .map(|e| g.note_rect(e).is_some_and(|r| r.intersects(rect)))
        .collect()
}

/// Sorts `events` by position, keeping the order of equal positions, and `sel` along with them.
pub fn sort(events: &mut Vec<Event>, sel: &mut Vec<bool>) {
    sel.resize(events.len(), false);
    let mut pairs: Vec<(Event, bool)> = events.drain(..).zip(sel.drain(..)).collect();
    pairs.sort_by_key(|(e, _)| e.pos());
    (*events, *sel) = pairs.into_iter().unzip();
}

/// Inserts a note after any at the same position and returns its index; it starts unselected.
pub fn add_note(
    events: &mut Vec<Event>,
    sel: &mut Vec<bool>,
    pos: f64,
    len: f64,
    key: u8,
) -> usize {
    let e = note_event(pos, len, key);
    let i = events.partition_point(|x| x.pos() <= e.pos());
    events.insert(i, e);
    sel.insert(i, false);
    i
}

/// Removes the selected events; false when nothing was selected.
pub fn remove_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>) -> bool {
    let before = events.len();
    let mut keep = sel.iter().map(|s| !s);
    events.retain(|_| keep.next().unwrap_or(true));
    sel.retain(|s| !s);
    events.len() != before
}

/// Applies `f(pos, len, key)` to the notes flagged in `which`.
fn map_notes(
    orig: &[Event],
    which: &[bool],
    f: impl Fn(f64, f64, u8) -> (f64, f64, u8),
) -> Vec<Event> {
    orig.iter()
        .zip(which)
        .map(|(e, &on)| match note(e) {
            Some((pos, len, key)) if on => {
                let (pos, len, key) = f(pos, len, key);
                with(*e, pos, len, key)
            }
            _ => *e,
        })
        .collect()
}

/// The flagged notes of `which` among `orig`, as `(pos, len, key)`.
pub(crate) fn flagged<'a>(
    orig: &'a [Event],
    which: &'a [bool],
) -> impl Iterator<Item = (f64, f64, u8)> + 'a {
    orig.iter()
        .zip(which)
        .filter(|(_, on)| **on)
        .filter_map(|(e, _)| note(e))
}

/// `orig` with the flagged notes moved by `dt` ticks and `dk` keys. Both are clamped for the group, so that every
/// moved note stays at tick ≥ 0 and inside the key range and the group keeps its shape. Not re-sorted.
pub fn shift_notes(orig: &[Event], which: &[bool], dt: f64, dk: i32) -> Vec<Event> {
    let Some(min_pos) = flagged(orig, which).map(|(pos, _, _)| pos).reduce(f64::min) else {
        return orig.to_vec();
    };
    let dt = dt.max(-min_pos);
    let (down, up) = transpose_range(orig, which);
    // `max`/`min`, not `clamp`: no panic even if the bounds ever crossed.
    let dk = dk.max(down).min(up);
    map_notes(orig, which, |pos, len, key| {
        (pos + dt, len, (i32::from(key) + dk) as u8)
    })
}

/// `orig` with the flagged notes lengthened by `dl` ticks, each at least `min_len` long. Not re-sorted.
pub fn stretch_notes(orig: &[Event], which: &[bool], dl: f64, min_len: f64) -> Vec<Event> {
    map_notes(orig, which, |pos, len, key| {
        (pos, (len + dl).max(min_len), key)
    })
}

/// The flagged notes' span in ticks: (earliest start, latest end).
fn span(orig: &[Event], which: &[bool]) -> Option<(f64, f64)> {
    flagged(orig, which)
        .map(|(pos, len, _)| (pos, pos + len))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// The selection's end capsule dragged to tick `finger`: the box's end goes to [`box_end`], and every flagged note's
/// length changes by as much as the box's width. Only the box has a minimum; a note is only kept at least a tick long.
/// Not re-sorted.
pub fn stretch_to(orig: &[Event], which: &[bool], finger: f64, grid: Option<f64>) -> Vec<Event> {
    let Some((start, end)) = span(orig, which) else {
        return orig.to_vec();
    };
    stretch_notes(orig, which, box_end(finger, start, grid) - end, 1.0)
}

/// The selection's ↔ bar dragged to tick `finger`: the box's end goes to [`box_end`] and the flagged notes scale in
/// time about the box's start A by k = new width / width (1 for no width): start' = A + (start − A)·k (on whole
/// ticks), length' = length·k. Not re-sorted.
pub fn scale_notes(orig: &[Event], which: &[bool], finger: f64, grid: Option<f64>) -> Vec<Event> {
    let Some((a, end)) = span(orig, which) else {
        return orig.to_vec();
    };
    let k = if end > a {
        (box_end(finger, a, grid) - a) / (end - a)
    } else {
        1.0
    };
    map_notes(orig, which, |pos, len, key| {
        (a + (pos - a) * k, len * k, key)
    })
}

/// Copies the selected notes (velocity, pan and slide too) to just after the selection (its span rounded up to
/// `grid`); the copies become the selection.
pub fn duplicate_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>, grid: f64) {
    let (mut start, mut end) = (f64::MAX, f64::MIN);
    for (pos, len, _) in flagged(events, sel) {
        start = start.min(pos);
        end = end.max(pos + len);
    }
    if start == f64::MAX {
        return;
    }
    let interval = ((end - start) / grid).ceil().max(1.0) * grid;
    let copies: Vec<Event> = events
        .iter()
        .zip(sel.iter())
        .filter(|(_, s)| **s)
        .filter_map(|(e, _)| note(e).map(|(pos, len, key)| with(*e, pos + interval, len, key)))
        .collect();
    sel.iter_mut().for_each(|s| *s = false);
    sel.extend(copies.iter().map(|_| true));
    events.extend(copies);
    sort(events, sel);
}

/// Moves the selected notes' starts to the nearest `grid` line; false when nothing moved.
pub fn quantize_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>, grid: f64) -> bool {
    let new = map_notes(events, sel, |pos, len, key| {
        (snap_round(pos, Some(grid)), len, key)
    });
    if new == *events {
        return false;
    }
    *events = new;
    sort(events, sel);
    true
}

/// Lengthens each selected note to the next later start among the selected notes; the last ones keep theirs.
pub fn legato_selected(events: &mut [Event], sel: &[bool]) {
    let starts: Vec<f64> = flagged(events, sel).map(|(pos, _, _)| pos).collect();
    let new = map_notes(events, sel, |pos, len, key| {
        let next = starts.iter().copied().filter(|&s| s > pos).reduce(f64::min);
        (pos, next.map_or(len, |s| s - pos), key)
    });
    events.copy_from_slice(&new);
}

/// Adds a copy of each selected note `dk` keys away (skipping those that would leave the key range); the
/// copies start unselected.
pub fn double_selected(events: &mut Vec<Event>, sel: &mut Vec<bool>, dk: i32) {
    let copies: Vec<Event> = events
        .iter()
        .zip(sel.iter())
        .filter(|(_, s)| **s)
        .filter_map(|(e, _)| {
            let (pos, len, key) = note(e)?;
            let key = u8::try_from(i32::from(key) + dk)
                .ok()
                .filter(|&k| k <= TOP_KEY)?;
            Some(with(*e, pos, len, key))
        })
        .collect();
    sel.extend(copies.iter().map(|_| false));
    events.extend(copies);
    sort(events, sel);
}

/// Sets every selected note to `len` ticks.
pub fn set_length_selected(events: &mut [Event], sel: &[bool], len: f64) {
    let new = map_notes(events, sel, |pos, _, key| (pos, len, key));
    events.copy_from_slice(&new);
}

/// The flags of the notes a note-menu item acts on: the selection, or every note when nothing is selected.
pub fn targets(events: &[Event], sel: &[bool]) -> Vec<bool> {
    if sel.contains(&true) {
        (0..events.len())
            .map(|i| sel.get(i) == Some(&true))
            .collect()
    } else {
        events.iter().map(|e| note(e).is_some()).collect()
    }
}

/// The key offsets `(down, up)` (down ≤ 0 ≤ up) that keep every flagged note inside the key range; `(0, 0)`
/// without notes.
pub fn transpose_range(events: &[Event], which: &[bool]) -> (i32, i32) {
    let keys = || flagged(events, which).map(|(_, _, key)| i32::from(key));
    match (keys().min(), keys().max()) {
        (Some(low), Some(high)) => (-low, i32::from(TOP_KEY) - high),
        _ => (0, 0),
    }
}

/// The transpose popup's offset: `%+d` above zero, `%d` otherwise.
pub fn offset_label(offset: i32) -> String {
    if offset > 0 {
        format!("{offset:+}")
    } else {
        offset.to_string()
    }
}

/// Flips the slide flag of each flagged note.
pub fn toggle_slide(events: &mut [Event], which: &[bool]) {
    for (e, _) in events.iter_mut().zip(which).filter(|(_, on)| **on) {
        if let Event::Note { key, slide, .. } = e
            && *key <= TOP_KEY
        {
            *slide = !*slide;
        }
    }
}

/// The project scale: the song's key lock as the keyboard settings show it, as its root (0 = C) and its
/// [`SCALE_MASKS`] mask. A root outside 0..12 reads as C, a stored scale without a picker entry as Chromatic.
pub fn project_scale(song: &Song) -> (u8, u16) {
    let root = u8::try_from(song.key_root)
        .ok()
        .filter(|&r| r < 12)
        .unwrap_or(0);
    (root, SCALE_MASKS[scale_from_stored(song.key_scale)])
}

/// `key` is a tone of the scale `mask` (most significant bit = the root) on `root`.
pub(crate) fn in_scale(key: u8, root: u8, mask: u16) -> bool {
    let degree = (i32::from(key) - i32::from(root)).rem_euclid(12);
    mask >> (11 - degree) & 1 == 1
}

/// The scale tone nearest `key` inside the key range, the lower of two equally near ones; `key` itself for an empty
/// mask.
pub fn snap_key(key: u8, root: u8, mask: u16) -> u8 {
    (0..12)
        .flat_map(|d| {
            [
                key.checked_sub(d),
                key.checked_add(d).filter(|&k| k <= TOP_KEY),
            ]
        })
        .flatten()
        .find(|&k| in_scale(k, root, mask))
        .unwrap_or(key)
}

/// `Snap to Scale`: moves each flagged note to the nearest tone of the scale `mask` on `root` ([`snap_key`]).
pub fn snap_to_scale(events: &mut [Event], which: &[bool], root: u8, mask: u16) {
    let new = map_notes(events, which, |pos, len, key| {
        (pos, len, snap_key(key, root, mask))
    });
    events.copy_from_slice(&new);
}

/// A `Guess Scale...` candidate: a root and a [`SCALES`] scale, and how the analysed notes fit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guess {
    pub root: u8,
    pub scale: usize,
    /// Notes outside the scale.
    pub accidentals: usize,
    pub in_scale: usize,
    /// Notes on the root's pitch class.
    pub roots: usize,
}

impl Guess {
    /// Its result row, `rank` from 1: `#%d %s %s (%s)`, the root by the key picker's sharp name.
    pub fn label(&self, rank: usize) -> String {
        let root = KEYS[usize::from(self.root)]
            .split(" / ")
            .next()
            .unwrap_or_default();
        let detail = if self.accidentals == 0 {
            format!("All in scale, {} root notes", self.roots)
        } else {
            format!(
                "{} accidentals, {} in scale, {} root notes",
                self.accidentals, self.in_scale, self.roots
            )
        };
        format!("#{rank} {root} {} ({detail})", SCALES[self.scale])
    }
}

/// Every root and scale but Chromatic (which fits anything), against the flagged notes, each note counted once: best
/// first by fewest accidentals, then most root notes, then picker order (scale, then root). `None` with fewer than
/// three different pitch classes.
pub fn guess_scales(events: &[Event], which: &[bool]) -> Option<Vec<Guess>> {
    let keys: Vec<u8> = flagged(events, which).map(|(_, _, key)| key).collect();
    let classes = keys.iter().fold(0u16, |m, k| m | 1 << (k % 12));
    if classes.count_ones() < 3 {
        return None;
    }
    let mut guesses: Vec<Guess> = (0..SCALES.len())
        .filter(|&scale| scale != CHROMATIC)
        .flat_map(|scale| (0..12).map(move |root| (scale, root)))
        .map(|(scale, root)| {
            let inside = (keys.iter())
                .filter(|&&k| in_scale(k, root, SCALE_MASKS[scale]))
                .count();
            Guess {
                root,
                scale,
                accidentals: keys.len() - inside,
                in_scale: inside,
                roots: keys.iter().filter(|&&k| k % 12 == root).count(),
            }
        })
        .collect();
    guesses.sort_by_key(|g| (g.accidentals, Reverse(g.roots)));
    Some(guesses)
}

/// The `Humanize` panel's settings ([`super::humanize`]). The panel is issue #29's;
/// its defaults are the plain `Humanize` rule, so `Apply` on an untouched panel does what the item did before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Humanize {
    /// The start's deviation σ, in beats (spec: 0.025).
    pub timing: f64,
    /// The length's σ, a share of the note's length (spec: 0).
    pub length: f64,
    /// The velocity's σ, a share of full scale (spec: 0.05).
    pub velocity: f64,
    /// `Velocity to Length` and `Pitch to Length`, −1..=1: at +1 the loudest (or highest) note is half as long and the
    /// quietest (or lowest) twice, at −1 the other way round ([`humanize_with`]).
    pub vel_to_len: f64,
    pub pitch_to_len: f64,
    /// The base adjustments, made before the deviations: a velocity offset (a share of full scale), a length factor
    /// and a start offset in whole ticks (1/128 beat).
    pub vel_offset: f64,
    pub len_scale: f64,
    pub time_offset: f64,
    /// `Apply to`: every note of the clip, not only the selected ones.
    pub all: bool,
}

impl Default for Humanize {
    /// The spec's rule on the selected notes.
    fn default() -> Self {
        Self {
            timing: 0.025,
            length: 0.0,
            velocity: 0.05,
            vel_to_len: 0.0,
            pitch_to_len: 0.0,
            vel_offset: 0.0,
            len_scale: 1.0,
            time_offset: 0.0,
            all: false,
        }
    }
}

/// [`humanize_with`] drawing from [`normal`] seeded by `seed`: the same seed and settings give the same notes.
pub fn humanize(events: &[Event], sel: &[bool], s: &Humanize, seed: u64) -> Vec<Event> {
    // Spread small seeds over the generator's state.
    humanize_with(
        events,
        sel,
        s,
        normal(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
    )
}

/// `Humanize`: `events` with each target note (a selected one, or every one with [`Humanize::all`]) changed once,
/// where `normal` draws N(0, 1), three draws a note whatever the settings, so that a slider scales the same
/// deviations:
/// - start = max(0, start + offset + N(0, σt beat)), truncated to whole ticks (1/128 beat);
/// - velocity v = clamp(v + 255·offset, 0, 255), then clamp(trunc(v + 255·N(0, σv)), 1, 255);
/// - length = max(1 tick, length · scale · 2^(−a·(2v/255 − 1)) · 2^(−b·(2p − 1)) · (1 + N(0, σl))), with v that offset
///   velocity, a = `Velocity to Length`, b = `Pitch to Length` and p the key's place in the target notes' key range
///   (0 lowest, 1 highest, ½ for a single key).
///
/// Key, pan and slide stay. The notes are neither re-sorted nor deselected: the order stays, so the preview keeps the
/// selection; `Apply` re-sorts. With the defaults this is the plain `Humanize` rule, drawing start then
/// velocity as it does.
pub fn humanize_with(
    events: &[Event],
    sel: &[bool],
    s: &Humanize,
    mut normal: impl FnMut() -> f64,
) -> Vec<Event> {
    let target = |i: usize, e: &Event| note(e).is_some() && (s.all || sel.get(i) == Some(&true));
    let keys = (events.iter().enumerate())
        .filter(|&(i, e)| target(i, e))
        .filter_map(|(_, e)| note(e).map(|n| f64::from(n.2)));
    let (low, high) = keys.fold((f64::MAX, f64::MIN), |(l, h), k| (l.min(k), h.max(k)));
    let place = |key: u8| match high > low {
        true => (f64::from(key) - low) / (high - low),
        false => 0.5,
    };
    (events.iter().enumerate())
        .map(|(i, e)| match *e {
            Event::Note {
                pos,
                len_beats,
                key,
                velocity,
                pan,
                slide,
            } if target(i, e) => {
                let (zt, zv, zl) = (normal(), normal(), normal());
                let v = (f64::from(velocity) + s.vel_offset * 255.0).clamp(0.0, 255.0);
                let k = s.len_scale
                    * (-s.vel_to_len * (2.0 * v / 255.0 - 1.0)).exp2()
                    * (-s.pitch_to_len * (2.0 * place(key) - 1.0)).exp2();
                let len = (len_beats * BEAT * k * (1.0 + zl * s.length)).max(1.0);
                // `as` truncates (and saturates). `v + 255σ` is `(v/255 + σ)·255` without the rounding of `v/255`.
                Event::Note {
                    pos: (f64::from(pos) + s.time_offset + zt * s.timing * BEAT).max(0.0) as u32,
                    len_beats: len / BEAT,
                    key,
                    velocity: (v + zv * s.velocity * 255.0).trunc().clamp(1.0, 255.0) as u8,
                    pan,
                    slide,
                }
            }
            other => other,
        })
        .collect()
}

/// Standard normal draws: Marsaglia's polar method (one value of each pair) over xorshift64*.
pub fn normal(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed | 1;
    let mut uniform = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let bits = state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
        bits as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    move || loop {
        let (u, v) = (uniform(), uniform());
        let s = u * u + v * v;
        if s > 0.0 && s < 1.0 {
            return u * (-2.0 * s.ln() / s).sqrt();
        }
    }
}

/// The loop length (beats, whole bars) that holds every note's end; 0 without notes.
pub fn needed_loop_beats(events: &[Event], beats_per_bar: u32) -> f64 {
    let end = events
        .iter()
        .filter_map(note)
        .map(|(pos, len, _)| pos + len)
        .fold(0.0, f64::max);
    let bar = BEAT * f64::from(beats_per_bar);
    (end / bar).ceil() * f64::from(beats_per_bar)
}

/// The clip's loop and first-play window in ticks: `(start, loop, visible, offset)`, `None` without a loop
/// ([`Clip::loop_ticks`]).
fn window(clip: &Clip) -> Option<(f64, f64, f64, f64)> {
    let lp = clip.loop_ticks()?;
    let visible = if clip.visible_len_beats > 0.0 {
        clip.visible_len_beats
    } else {
        clip.loop_len_beats
    } * BEAT;
    Some((
        f64::from(clip.start_tick),
        lp,
        visible,
        clip.offset_beats * BEAT,
    ))
}

/// The content tick that plays at `song_tick`, if the clip plays then.
pub fn content_tick(clip: Option<&Clip>, song_tick: u64) -> Option<f64> {
    let (start, lp, visible, offset) = window(clip?)?;
    let rel = song_tick as f64 - start;
    (0.0..visible)
        .contains(&rel)
        .then(|| (rel + offset).rem_euclid(lp))
}

/// A note of another channel drawn faintly behind the edited clip's notes, at the content tick it plays against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ghost {
    pub pos: f64,
    pub len: f64,
    pub key: u8,
    /// Its channel's hue.
    pub hue: f32,
}

/// The notes of every other channel's notes lanes that play during the edited clip's first loop pass (without a
/// clip: during the first bar), mapped into the clip's content: those that reach the content ticks `view` shows, from
/// clips whose loop passes are at least `px` ticks (a pixel) long ([`Clip::plays_to_draw`]). So the walk is bounded
/// by the view, not by a file's loop or visible lengths. A hidden track's notes (`Hide track`) are never ghosts, whatever
/// Settings `Show hidden tracks` (the playlist's) says.
pub fn ghost_notes(
    song: &Song,
    channel: usize,
    clip: Option<&Clip>,
    beats_per_bar: u32,
    view: std::ops::Range<f64>,
    px: f64,
) -> Vec<Ghost> {
    let bar = BEAT * f64::from(beats_per_bar);
    let (start, lp, visible, offset) = clip.and_then(window).unwrap_or((0.0, bar, bar, 0.0));
    let end = start + visible.min(lp);
    // Song tick s of the pass plays at content u mod lp, u = s − start + off running over [off, off + lp).
    let off = offset.rem_euclid(lp);
    let mut ghosts = Vec::new();
    for (ci, ch) in song.channels.iter().enumerate() {
        if ci == channel {
            continue;
        }
        for lane in (ch.lanes.iter()).filter(|l| l.kind == LaneKind::Notes && !l.hidden) {
            for c in &lane.clips {
                // A note that starts before the view reaches into it by its length.
                // ponytail: at most a view's width; a ghost longer than that, starting further before the view, is
                // left out.
                let longest = (c.events.iter().filter_map(note))
                    .map(|(_, len, _)| len)
                    .fold(0.0, f64::max)
                    .min(view.end - view.start);
                let from = view.start - longest;
                // u in [from, view.end) or a loop later: two spans, or the whole pass when the view holds a loop.
                let spans = match view.end - from < lp {
                    true => [0.0, lp].map(|k| (start - off + from + k, start - off + view.end + k)),
                    false => [(start, end), (0.0, 0.0)],
                };
                for (a, b) in spans {
                    let (a, b) = (a.max(start), b.min(end));
                    if a >= b {
                        continue;
                    }
                    for (tick, e) in c.plays_to_draw(a.floor() as u64, b.ceil() as u64, px) {
                        if let Some((_, len, key)) = note(e) {
                            ghosts.push(Ghost {
                                pos: (tick as f64 - start + offset).rem_euclid(lp),
                                len,
                                key,
                                hue: ch.hue,
                            });
                        }
                    }
                }
            }
        }
    }
    ghosts
}
