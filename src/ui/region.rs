//! The time-selection (region) menu and its operations on song data. A range is in its ruler's ticks: song ticks in
//! the playlist, content ticks of the open clip in the piano roll and the drum editor. The piano roll and the drum
//! editor act on every note or hit of the open clip, whatever the selection; the playlist on every clip of every lane
//! of every channel. Notes, hits and clips across an edge an operation cuts at are split there (a clip's right piece
//! starts that much later into its content, so it plays on seamlessly). The playhead is in whole ticks, which in the
//! playlist truncates s, e and p to 1/128 beat.
//!
//! A clip editor's op fits the loop to the content ([`Clip::fit_loop`], the generic clip-update rule: the smallest
//! bar·2ᵏ beats holding every event's end, so it may shrink; an event lies inside its loop in this model);
//! zero-length items (automation points) are never split.

use crate::song::{Channel, Clip, Event, PPQ};
use crate::ui::clips::{at_pos, end_tick};
use crate::ui::piano_roll::notes::content_tick;
use crate::ui::popup::PopupItem;
use crate::ui::preview;

/// A time range picked on a ruler, `start` < `end`, in the ruler's ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    pub start: u32,
    pub end: u32,
}

impl TimeRange {
    pub fn contains(self, tick: f64) -> bool {
        f64::from(self.start) <= tick && tick < f64::from(self.end)
    }

    /// The song's play loop on the ruler of a clip editor whose content tick 0 is song tick `origin`: the part of it at
    /// or after that tick.
    pub fn in_clip(play_loop: Option<(u32, u32)>, origin: i64) -> Option<Self> {
        let (start, end) = play_loop?;
        let at = |tick: u32| (i64::from(tick) - origin).clamp(0, u32::MAX.into()) as u32;
        let end = Some(at(end)).filter(|&e| e > 0)?;
        Some(Self {
            start: at(start),
            end,
        })
    }

    /// This range of a clip editor whose content tick 0 is song tick `origin`, as the song's play loop: none when it
    /// lies wholly before the song start.
    pub fn song_loop(self, origin: i64) -> Option<(u32, u32)> {
        let at = |tick: u32| (origin + i64::from(tick)).clamp(0, u32::MAX.into()) as u32;
        Some((at(self.start), at(self.end))).filter(|(a, b)| a < b)
    }

    /// One flag per event: it starts in the range.
    pub fn flags(self, events: &[Event]) -> Vec<bool> {
        events
            .iter()
            .map(|e| self.contains(e.pos().into()))
            .collect()
    }
}

/// The song tick a clip editor's content tick 0 plays at: the clip's start less its offset, a loop later when that is
/// before the song start (Preview's span: still negative when the offset passes the start by more than a loop); 0
/// without a clip.
pub fn clip_origin(clip: Option<&Clip>) -> i64 {
    clip.and_then(preview::span).map_or(0, |(start, _)| start)
}

/// The content tick a clip editor shows the transport's song tick `tick` at, if the clip plays then: while Preview
/// plays the clip (`previewing`), `tick` − s over its span [s, e) ([`preview::solo`] plays the content from s); else
/// where the clip plays in the song ([`content_tick`]).
pub fn playhead_tick(clip: Option<&Clip>, tick: u64, previewing: bool) -> Option<f64> {
    match clip.and_then(preview::span).filter(|_| previewing) {
        Some((s, e)) => {
            let tick = tick as i64;
            (s..e).contains(&tick).then(|| (tick - s) as f64)
        }
        None => content_tick(clip, tick),
    }
}

/// The song tick a tap on a clip editor's ruler at content tick `tick` seeks to, so that [`playhead_tick`] reads back
/// there: while Preview plays the clip (`previewing`), its [`clip_origin`] s plus `tick`; else where the clip first
/// plays that content ([`Clip::song_tick`]), and outside the clip's window s plus `tick` again; never before 0.
pub fn seek_tick(clip: Option<&Clip>, tick: u32, previewing: bool) -> u64 {
    (clip.filter(|_| !previewing).and_then(|c| c.song_tick(tick)))
        .unwrap_or((clip_origin(clip) + i64::from(tick)).max(0) as u64)
}

/// The region menu's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionOp {
    Deselect,
    InsertSpace,
    Duplicate,
    CopyToPlayhead,
    MoveToPlayhead,
    Delete,
    DeleteSpace,
    Trim,
    SnapToScale,
    GuessScale,
    /// The audio clip editor's own items ([`audio_items`]); the generic editors never get them.
    InsertSilence,
    Clear,
    Normalize,
    FadeIn,
    FadeOut,
}

impl RegionOp {
    /// The range is the song's play loop: every item but `Duplicate` clears it.
    pub fn keeps_loop(self) -> bool {
        self == Self::Duplicate
    }
}

const OPS: [(RegionOp, &str); 10] = [
    (RegionOp::Deselect, "Deselect"),
    (RegionOp::InsertSpace, "Insert space"),
    (RegionOp::Duplicate, "Duplicate"),
    (RegionOp::CopyToPlayhead, "Copy to playhead"),
    (RegionOp::MoveToPlayhead, "Move to playhead"),
    (RegionOp::Delete, "Delete"),
    (RegionOp::DeleteSpace, "Delete space"),
    (RegionOp::Trim, "Trim selection"),
    (RegionOp::SnapToScale, "Snap to Scale"),
    (RegionOp::GuessScale, "Guess Scale..."),
];

/// The region menu for `range` with the playhead at tick `playhead`: one column, `Deselect` first. `Copy to playhead`
/// and `Move to playhead` are enabled only while the playhead is outside the range (and somewhere in the clip:
/// `None` in a clip editor whose clip the transport is not in), the rest always. `scale_tools` adds the piano roll's
/// `Snap to Scale` and `Guess Scale...` at the bottom of column 0.
pub fn items(
    range: TimeRange,
    playhead: Option<f64>,
    scale_tools: bool,
) -> Vec<PopupItem<RegionOp>> {
    let n = if scale_tools { OPS.len() } else { 8 };
    let at_playhead = |op| matches!(op, RegionOp::CopyToPlayhead | RegionOp::MoveToPlayhead);
    let outside = playhead.is_some_and(|p| !range.contains(p));
    OPS[..n]
        .iter()
        .enumerate()
        .map(|(row, &(op, label))| PopupItem {
            disabled: at_playhead(op) && !outside,
            ..PopupItem::new(op, 0, row, label)
        })
        .collect()
}

/// The audio clip editor's region menu for `range` with the playhead at tick `playhead`: column 0 `Deselect`, `Insert
/// silence`, `Duplicate`, `Copy to playhead`, `Move to playhead`, `Clear`, `Delete space`, `Trim selection`; column 1
/// `Normalize`, `Fade In`, `Fade Out`. The playhead items are enabled as in [`items`].
pub fn audio_items(range: TimeRange, playhead: Option<f64>) -> Vec<PopupItem<RegionOp>> {
    use RegionOp::*;
    let cols: [&[(RegionOp, &'static str)]; 2] = [
        &[
            (Deselect, "Deselect"),
            (InsertSilence, "Insert silence"),
            (Duplicate, "Duplicate"),
            (CopyToPlayhead, "Copy to playhead"),
            (MoveToPlayhead, "Move to playhead"),
            (Clear, "Clear"),
            (DeleteSpace, "Delete space"),
            (Trim, "Trim selection"),
        ],
        &[
            (Normalize, "Normalize"),
            (FadeIn, "Fade In"),
            (FadeOut, "Fade Out"),
        ],
    ];
    let outside = playhead.is_some_and(|p| !range.contains(p));
    (cols.iter().enumerate())
        .flat_map(|(col, ops)| {
            ops.iter()
                .enumerate()
                .map(move |(row, &(op, label))| PopupItem {
                    disabled: matches!(op, CopyToPlayhead | MoveToPlayhead) && !outside,
                    ..PopupItem::new(op, col, row, label)
                })
        })
        .collect()
}

/// `op` on every note or hit of `clip` for `range` (content ticks), with the playhead at content tick `playhead`,
/// in bars of `beats_per_bar`. `Trim selection`'s end is the clip's loop length.
pub fn edit_clip(
    clip: &mut Clip,
    op: RegionOp,
    range: TimeRange,
    playhead: u32,
    beats_per_bar: u32,
) {
    let end = clip.loop_len_beats * f64::from(PPQ);
    apply(&mut clip.events, op, range, playhead, end);
    // Stable: what shares a position keeps its order.
    clip.events.sort_by_key(Event::pos);
    clip.fit_loop(beats_per_bar);
}

/// `op` on every clip of every lane of `channels` for `range` (song ticks), with the playhead at song tick
/// `playhead`, in bars of `beats_per_bar`. `Trim selection`'s end is the song's length: the last clip's end, rounded
/// up to a whole bar.
pub fn edit_lanes(
    channels: &mut [Channel],
    op: RegionOp,
    range: TimeRange,
    playhead: u32,
    beats_per_bar: u32,
) {
    let lanes = || channels.iter().flat_map(|c| &c.lanes);
    let last = (lanes().flat_map(|l| &l.clips))
        .map(end_tick)
        .fold(0.0, f64::max);
    let bar = f64::from(beats_per_bar.max(1) * PPQ);
    let end = (last / bar).ceil() * bar;
    for lane in channels.iter_mut().flat_map(|c| &mut c.lanes) {
        apply(&mut lane.clips, op, range, playhead, end);
        // Pieces and copies come without an id.
        for clip in lane.clips.iter_mut().filter(|c| c.id == 0) {
            lane.last_clip_id += 1;
            clip.id = lane.last_clip_id;
        }
    }
}

/// What the operations move, split and copy: notes and hits in a clip, clips in a lane.
trait Item: Clone {
    fn start(&self) -> f64;
    fn end(&self) -> f64;
    /// The item moved to start at `start`.
    fn moved(&self, start: f64) -> Self;
    /// A new item: the same, starting at `start`.
    fn copy_at(&self, start: f64) -> Self {
        self.moved(start)
    }
    /// Keeps [start, `at`) and returns the rest, from `at`, which it is strictly inside.
    fn split_off(&mut self, at: f64) -> Self;
}

impl Item for Event {
    fn start(&self) -> f64 {
        self.pos().into()
    }

    fn end(&self) -> f64 {
        self.start() + len_beats(self) * f64::from(PPQ)
    }

    fn moved(&self, start: f64) -> Self {
        at_pos(*self, start as u32)
    }

    fn split_off(&mut self, at: f64) -> Self {
        let (start, end) = (self.start(), self.end());
        let mut rest = self.moved(at);
        set_len(self, (at - start) / f64::from(PPQ));
        set_len(&mut rest, (end - at) / f64::from(PPQ));
        rest
    }
}

fn len_beats(e: &Event) -> f64 {
    match *e {
        Event::Note { len_beats, .. } | Event::Drum { len_beats, .. } => len_beats,
        Event::Automation { .. } | Event::Envelope { .. } => 0.0,
    }
}

fn set_len(e: &mut Event, beats: f64) {
    if let Event::Note { len_beats, .. } | Event::Drum { len_beats, .. } = e {
        *len_beats = beats;
    }
}

impl Item for Clip {
    fn start(&self) -> f64 {
        self.start_tick.into()
    }

    fn end(&self) -> f64 {
        end_tick(self)
    }

    fn moved(&self, start: f64) -> Self {
        Self {
            start_tick: start as u32,
            ..self.clone()
        }
    }

    /// Id 0: [`edit_lanes`] gives it one.
    fn copy_at(&self, start: f64) -> Self {
        Self {
            id: 0,
            ..self.moved(start)
        }
    }

    /// The right piece plays on seamlessly: it starts that much later into the content (as `Slice` cuts).
    fn split_off(&mut self, at: f64) -> Self {
        let (ppq, end) = (f64::from(PPQ), self.end());
        let head = (at - self.start()) / ppq;
        let offset = self.offset_beats + head;
        let rest = Self {
            visible_len_beats: (end - at) / ppq,
            offset_beats: if self.loop_len_beats > 0.0 {
                offset.rem_euclid(self.loop_len_beats)
            } else {
                offset
            },
            ..self.copy_at(at)
        };
        self.visible_len_beats = head;
        rest
    }
}

/// The Semantics table's rule for `op` on `items`: s, e = the range, L = e − s, p = the playhead, `end` the content's
/// end (for `Trim selection`). `Deselect`, the scale tools and the audio items change nothing here.
fn apply<T: Item>(items: &mut Vec<T>, op: RegionOp, range: TimeRange, playhead: u32, end: f64) {
    let (s, e, p) = (
        f64::from(range.start),
        f64::from(range.end),
        f64::from(playhead),
    );
    let len = e - s;
    match op {
        RegionOp::InsertSpace => {
            split(items, s);
            shift(items, s, len);
        }
        RegionOp::Delete => delete(items, s, e),
        RegionOp::DeleteSpace => delete_space(items, s, e),
        RegionOp::Duplicate => {
            split(items, e);
            shift(items, e, len);
            split(items, s);
            let copies = copies(items, s, e, e);
            items.extend(copies);
        }
        RegionOp::CopyToPlayhead | RegionOp::MoveToPlayhead => {
            // An insert at p, which moves the source on if it lay after p.
            split(items, p);
            shift(items, p, len);
            let from = if p < s { s + len } else { s };
            split(items, from);
            split(items, from + len);
            let copies = copies(items, from, from + len, p);
            items.extend(copies);
            if op == RegionOp::MoveToPlayhead {
                delete_space(items, from, from + len);
            }
        }
        RegionOp::Trim => {
            if end > e {
                delete_space(items, e, end);
            }
            if s > 0.0 {
                delete_space(items, 0.0, s);
            }
        }
        RegionOp::Deselect
        | RegionOp::SnapToScale
        | RegionOp::GuessScale
        | RegionOp::InsertSilence
        | RegionOp::Clear
        | RegionOp::Normalize
        | RegionOp::FadeIn
        | RegionOp::FadeOut => {}
    }
}

/// Splits what `at` is strictly inside of there.
fn split<T: Item>(items: &mut Vec<T>, at: f64) {
    let rests: Vec<T> = (items.iter_mut())
        .filter(|i| i.start() < at && at < i.end())
        .map(|i| i.split_off(at))
        .collect();
    items.extend(rests);
}

/// Moves what starts at or after `from` by `by` ticks.
fn shift<T: Item>(items: &mut [T], from: f64, by: f64) {
    for item in items.iter_mut().filter(|i| i.start() >= from) {
        *item = item.moved(item.start() + by);
    }
}

fn inside<T: Item>(item: &T, s: f64, e: f64) -> bool {
    s <= item.start() && item.end() <= e
}

/// `Delete`: what lies inside [s, e] goes, what crosses an edge is cut there.
fn delete<T: Item>(items: &mut Vec<T>, s: f64, e: f64) {
    split(items, s);
    split(items, e);
    items.retain(|i| !inside(i, s, e));
}

/// `Delete space`: `Delete`, then what starts at or after s moves back by e − s.
fn delete_space<T: Item>(items: &mut Vec<T>, s: f64, e: f64) {
    delete(items, s, e);
    shift(items, s, s - e);
}

/// New copies of what lies inside [s, e] (split there already), moved to start from `to`.
fn copies<T: Item>(items: &[T], s: f64, e: f64, to: f64) -> Vec<T> {
    (items.iter())
        .filter(|i| inside(*i, s, e))
        .map(|i| i.copy_at(i.start() - s + to))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{ChannelKind, Lane, LaneKind, Song};

    fn note(pos: u32, len: u32, key: u8) -> Event {
        Event::Note {
            pos,
            len_beats: f64::from(len) / f64::from(PPQ),
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }

    fn hit(pos: u32, pad: u8) -> Event {
        Event::Drum {
            pos,
            len_beats: 0.25,
            pad,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            value: 0x7FFF,
            repeat: 0,
        }
    }

    fn range(start: u32, end: u32) -> TimeRange {
        TimeRange { start, end }
    }

    // ---- the menu ----

    fn labels(items: &[PopupItem<RegionOp>]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_ref()).collect()
    }

    #[test]
    fn the_menu_lists_the_eight_operations_deselect_first_in_one_column() {
        let items = items(range(128, 256), Some(0.0), false);
        assert_eq!(
            labels(&items),
            [
                "Deselect",
                "Insert space",
                "Duplicate",
                "Copy to playhead",
                "Move to playhead",
                "Delete",
                "Delete space",
                "Trim selection"
            ]
        );
        for (row, item) in items.iter().enumerate() {
            assert_eq!((item.col, item.row), (0, row));
        }
    }

    #[test]
    fn the_piano_rolls_scale_tools_sit_at_the_bottom_of_column_0() {
        let items = items(range(128, 256), Some(0.0), true);
        assert_eq!(labels(&items)[8..], ["Snap to Scale", "Guess Scale..."]);
        assert_eq!(
            items[8..]
                .iter()
                .map(|i| (i.col, i.row))
                .collect::<Vec<_>>(),
            [(0, 8), (0, 9)]
        );
        assert_eq!(items[8].id, Some(RegionOp::SnapToScale));
        assert_eq!(items[9].id, Some(RegionOp::GuessScale));
    }

    #[test]
    fn the_playhead_items_are_enabled_only_with_the_playhead_outside_the_range() {
        let disabled = |playhead: Option<f64>| -> Vec<String> {
            items(range(128, 256), playhead, true)
                .into_iter()
                .filter(|i| i.disabled)
                .map(|i| i.label.into_owned())
                .collect()
        };
        for inside in [Some(128.0), Some(200.0), Some(255.5), None] {
            assert_eq!(
                disabled(inside),
                ["Copy to playhead", "Move to playhead"],
                "{inside:?}"
            );
        }
        // Before the range, at its end and after it: outside.
        for outside in [0.0, 127.9, 256.0, 1000.0] {
            assert!(disabled(Some(outside)).is_empty(), "{outside}");
        }
    }

    #[test]
    fn a_clip_editors_range_is_the_play_loop_moved_by_the_clips_origin() {
        // A clip whose content tick 0 plays at song tick 256.
        assert_eq!(
            TimeRange::in_clip(Some((384, 512)), 256),
            Some(range(128, 256))
        );
        assert_eq!(range(128, 256).song_loop(256), Some((384, 512)));
        // Starting before the origin: from content tick 0. Ending at or before it, or none: no range.
        assert_eq!(
            TimeRange::in_clip(Some((128, 384)), 256),
            Some(range(0, 128))
        );
        assert_eq!(TimeRange::in_clip(Some((128, 256)), 256), None);
        assert_eq!(TimeRange::in_clip(None, 256), None);
    }

    #[test]
    fn a_tap_seeks_where_the_clip_plays_the_content_else_from_its_origin() {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(512, 4.0);
        clip.offset_beats = 2.0;
        // Content 128 plays 384 ticks into the clip; content 0 two beats in.
        assert_eq!(seek_tick(Some(clip), 128, false), 896);
        assert_eq!(seek_tick(Some(clip), 0, false), 768);
        // Previewed, the content plays from the origin, 256.
        assert_eq!(seek_tick(Some(clip), 128, true), 384);
        // Shown a beat long, the clip plays content 256..384 only: the rest seeks from its origin, 256.
        clip.visible_len_beats = 1.0;
        assert_eq!(seek_tick(Some(clip), 300, false), 556);
        assert_eq!(seek_tick(Some(clip), 0, false), 256);
        // No clip: the tick.
        assert_eq!(seek_tick(None, 128, false), 128);
        assert_eq!(seek_tick(None, 128, true), 128);
    }

    #[test]
    fn the_playhead_reads_where_the_clip_plays_or_while_previewed_through_its_span() {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(512, 4.0);
        clip.offset_beats = 2.0;
        // In the song the clip plays content 256.. from 512; previewed, content 0.. from 256 to 768.
        assert_eq!(playhead_tick(Some(clip), 300, false), None);
        assert_eq!(playhead_tick(Some(clip), 600, false), Some(344.0));
        assert_eq!(playhead_tick(Some(clip), 300, true), Some(44.0));
        assert_eq!(playhead_tick(Some(clip), 600, true), Some(344.0));
        assert_eq!(playhead_tick(Some(clip), 768, true), None);
        // Shown a beat long: the song plays 512..640 only, Preview its whole span.
        clip.visible_len_beats = 1.0;
        assert_eq!(playhead_tick(Some(clip), 700, false), None);
        assert_eq!(playhead_tick(Some(clip), 700, true), Some(444.0));
        assert_eq!(playhead_tick(None, 700, true), None);
    }

    #[test]
    fn a_span_before_the_song_start_maps_from_a_negative_origin() {
        // Five beats into a one-bar clip at 0: Preview's span is [−128, 384) (`preview::span`).
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 4.0);
        clip.offset_beats = 5.0;
        assert_eq!(clip_origin(Some(clip)), -128);
        assert_eq!(playhead_tick(Some(clip), 0, true), Some(128.0));
        assert_eq!(playhead_tick(Some(clip), 384, true), None);
        // A tap seeks to s + content, never before 0.
        assert_eq!(seek_tick(Some(clip), 256, true), 128);
        assert_eq!(seek_tick(Some(clip), 64, true), 0);
        // The play loop on the ruler and back.
        assert_eq!(
            TimeRange::in_clip(Some((0, 256)), -128),
            Some(range(128, 384))
        );
        assert_eq!(range(0, 256).song_loop(-128), Some((0, 128)));
        assert_eq!(
            range(0, 128).song_loop(-128),
            None,
            "wholly before the song start"
        );
    }

    #[test]
    fn only_duplicate_keeps_the_play_loop() {
        for (op, _) in OPS {
            assert_eq!(op.keeps_loop(), op == RegionOp::Duplicate, "{op:?}");
        }
    }

    // ---- the clip editors: the Semantics table, on notes ----

    /// A one-bar clip, the range beat 1 (ticks 128..256), and a note in every place a note can lie against it
    /// (pos, len, key):
    /// A before it, B from before into it, C inside it, D from inside past it, E over all of it, F after it.
    fn bar() -> Clip {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 4.0);
        clip.events = vec![
            note(0, 64, 60),
            note(64, 128, 61),
            note(96, 256, 64),
            note(160, 32, 62),
            note(224, 64, 63),
            note(384, 64, 65),
        ];
        clip.clone()
    }

    const BEAT_1: TimeRange = TimeRange {
        start: 128,
        end: 256,
    };

    /// (pos, len in ticks, key) of every note, sorted.
    fn notes(clip: &Clip) -> Vec<(u32, u32, u8)> {
        let mut notes: Vec<_> = (clip.events.iter())
            .map(|e| match *e {
                Event::Note {
                    pos,
                    len_beats,
                    key,
                    ..
                } => (pos, (len_beats * f64::from(PPQ)).round() as u32, key),
                _ => unreachable!(),
            })
            .collect();
        notes.sort();
        notes
    }

    fn edited(op: RegionOp, playhead: u32) -> Clip {
        let mut clip = bar();
        edit_clip(&mut clip, op, BEAT_1, playhead, 4);
        // Sorted by position, as the clip keeps them.
        assert!(clip.events.is_sorted_by_key(Event::pos), "{op:?}");
        clip
    }

    #[test]
    fn insert_space_splits_at_the_start_and_moves_what_starts_there_or_later() {
        let clip = edited(RegionOp::InsertSpace, 0);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (256, 64, 61),
                (256, 224, 64),
                (288, 32, 62),
                (352, 64, 63),
                (512, 64, 65)
            ]
        );
        // The last note now ends in bar 2.
        assert_eq!(clip.loop_len_beats, 8.0);
    }

    #[test]
    fn delete_removes_what_is_inside_and_cuts_what_crosses_an_edge() {
        let clip = edited(RegionOp::Delete, 0);
        // B ends at s; D starts at e; E keeps [t, s) and [e, end); C goes.
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (256, 32, 63),
                (256, 96, 64),
                (384, 64, 65)
            ]
        );
        assert_eq!(clip.loop_len_beats, 4.0);
    }

    #[test]
    fn delete_space_deletes_then_moves_the_rest_back() {
        let clip = edited(RegionOp::DeleteSpace, 0);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (128, 32, 63),
                (128, 96, 64),
                (256, 64, 65)
            ]
        );
        assert_eq!(clip.loop_len_beats, 4.0, "one bar holds the rest");
        // From a longer loop, it shrinks to the smallest bar·2ᵏ that holds the rest.
        let mut long = bar();
        long.loop_len_beats = 16.0;
        edit_clip(&mut long, RegionOp::DeleteSpace, BEAT_1, 0, 4);
        assert_eq!(long.loop_len_beats, 4.0);
    }

    #[test]
    fn duplicate_copies_the_range_right_after_it_pushing_later_material() {
        let clip = edited(RegionOp::Duplicate, 0);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (128, 64, 61),
                (128, 128, 64),
                (160, 32, 62),
                (224, 32, 63),
                (256, 64, 61),
                (256, 128, 64),
                (288, 32, 62),
                (352, 32, 63),
                (384, 32, 63),
                (384, 96, 64),
                (512, 64, 65)
            ]
        );
    }

    #[test]
    fn copy_to_a_later_playhead_inserts_the_range_there() {
        let clip = edited(RegionOp::CopyToPlayhead, 512);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (128, 64, 61),
                (128, 128, 64),
                (160, 32, 62),
                (224, 32, 63),
                (256, 32, 63),
                (256, 96, 64),
                (384, 64, 65),
                (512, 64, 61),
                (512, 128, 64),
                (544, 32, 62),
                (608, 32, 63)
            ]
        );
        assert_eq!(clip.loop_len_beats, 8.0);
    }

    #[test]
    fn copy_to_an_earlier_playhead_copies_the_source_from_where_the_insert_moved_it() {
        let clip = edited(RegionOp::CopyToPlayhead, 0);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 61),
                (0, 128, 64),
                (32, 32, 62),
                (96, 32, 63),
                (128, 64, 60),
                (192, 64, 61),
                (224, 32, 64),
                (256, 64, 61),
                (256, 128, 64),
                (288, 32, 62),
                (352, 32, 63),
                (384, 32, 63),
                (384, 96, 64),
                (512, 64, 65)
            ]
        );
    }

    #[test]
    fn move_to_playhead_copies_then_deletes_the_space_of_the_moved_source() {
        let clip = edited(RegionOp::MoveToPlayhead, 0);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 61),
                (0, 128, 64),
                (32, 32, 62),
                (96, 32, 63),
                (128, 64, 60),
                (192, 64, 61),
                (224, 32, 64),
                (256, 32, 63),
                (256, 96, 64),
                (384, 64, 65)
            ]
        );
        // To a later playhead: the copy lands there, and the source's space closes before it.
        let clip = edited(RegionOp::MoveToPlayhead, 512);
        assert_eq!(
            notes(&clip),
            [
                (0, 64, 60),
                (64, 64, 61),
                (96, 32, 64),
                (128, 32, 63),
                (128, 96, 64),
                (256, 64, 65),
                (384, 64, 61),
                (384, 128, 64),
                (416, 32, 62),
                (480, 32, 63)
            ]
        );
    }

    #[test]
    fn trim_keeps_only_the_range_moved_to_the_start_and_the_clip_length() {
        let clip = edited(RegionOp::Trim, 0);
        assert_eq!(
            notes(&clip),
            [(0, 64, 61), (0, 128, 64), (32, 32, 62), (96, 32, 63)]
        );
        assert_eq!(clip.loop_len_beats, 4.0);
    }

    #[test]
    fn drum_hits_split_and_move_like_notes_and_only_a_new_clips_length_grows_with_the_loop() {
        // A new clip (visible length 0) shows its loop; a sized one keeps its length.
        let mut clip = bar();
        // A hit a step long from step 3 ends inside step 4: inserting at step 4 cuts it there.
        clip.events = vec![hit(0, 0), hit(96 + 16, 1), hit(480, 2)];
        edit_clip(&mut clip, RegionOp::InsertSpace, range(128, 192), 0, 4);
        let hits: Vec<(u32, f64, u8)> = (clip.events.iter())
            .map(|e| match *e {
                Event::Drum {
                    pos,
                    len_beats,
                    pad,
                    ..
                } => (pos, len_beats * f64::from(PPQ), pad),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            hits,
            [(0, 32.0, 0), (112, 16.0, 1), (192, 16.0, 1), (544, 32.0, 2)]
        );
        assert_eq!((clip.loop_len_beats, clip.visible_len_beats), (8.0, 0.0));
        assert_eq!(clip.visible_ticks(), 8.0 * f64::from(PPQ));
        let mut sized = bar();
        sized.visible_len_beats = 4.0;
        sized.events = vec![hit(0, 0), hit(96 + 16, 1), hit(480, 2)];
        edit_clip(&mut sized, RegionOp::InsertSpace, range(128, 192), 0, 4);
        assert_eq!((sized.loop_len_beats, sized.visible_len_beats), (8.0, 4.0));
    }

    // ---- the playlist ----

    /// Clips of notes that play at (tick, key): A at 0 (0:60, 256:62), B at 512 (512:64, 768:65), C at 1536
    /// (1536:67), each a bar long, on the Instrument (2); a drum clip at 1024 (pad 3) on the Drums (1).
    fn playlist() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::Instrument);
        let lane = &mut song.channels[2].lanes[0];
        lane.add_clip(0, 4.0).events = vec![note(0, 64, 60), note(256, 64, 62)];
        lane.add_clip(512, 4.0).events = vec![note(0, 64, 64), note(256, 64, 65)];
        lane.add_clip(1536, 4.0).events = vec![note(0, 64, 67)];
        song.channels[1].lanes[0].add_clip(1024, 4.0).events = vec![hit(0, 3)];
        song
    }

    /// Every play of channel `channel`'s main-lane clips as (song tick, key or pad), in time order.
    fn plays(song: &Song, channel: usize) -> Vec<(u64, u8)> {
        let mut plays: Vec<_> = (song.channels[channel].lanes[0].clips.iter())
            .flat_map(|c| {
                c.events_in_song_range(0, 100_000).map(|(t, e)| match *e {
                    Event::Note { key, .. } => (t, key),
                    Event::Drum { pad, .. } => (t, pad),
                    Event::Automation { .. } | Event::Envelope { .. } => (t, 0),
                })
            })
            .collect();
        plays.sort();
        plays
    }

    /// Beats 2 to 6: the second half of A and the first half of B.
    const MIDDLE: TimeRange = TimeRange {
        start: 256,
        end: 768,
    };

    fn after(op: RegionOp, playhead: u32) -> Song {
        let mut song = playlist();
        edit_lanes(&mut song.channels, op, MIDDLE, playhead, 4);
        // Every clip of a lane has an id of its own.
        for lane in song.channels.iter().flat_map(|c| &c.lanes) {
            let mut ids: Vec<u32> = lane.clips.iter().map(|c| c.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), lane.clips.len(), "{op:?}");
            assert!(ids.iter().all(|&id| id >= 1 && id <= lane.last_clip_id));
        }
        song
    }

    #[test]
    fn playlist_insert_space_splits_the_clip_across_the_start_and_moves_the_rest() {
        let song = after(RegionOp::InsertSpace, 0);
        assert_eq!(
            plays(&song, 2),
            [(0, 60), (768, 62), (1024, 64), (1280, 65), (2048, 67)]
        );
        // Every lane moves: the drums too.
        assert_eq!(plays(&song, 1), [(1536, 3)]);
    }

    #[test]
    fn playlist_delete_cuts_the_clips_across_both_edges() {
        let song = after(RegionOp::Delete, 0);
        assert_eq!(plays(&song, 2), [(0, 60), (768, 65), (1536, 67)]);
        assert_eq!(plays(&song, 1), [(1024, 3)]);
        let mut spans: Vec<(u32, f64, f64)> = (song.channels[2].lanes[0].clips.iter())
            .map(|c| (c.start_tick, c.visible_ticks(), c.offset_beats))
            .collect();
        spans.sort_by_key(|s| s.0);
        // B's right piece starts 2 beats into its content.
        assert_eq!(
            spans,
            [(0, 256.0, 0.0), (768, 256.0, 2.0), (1536, 512.0, 0.0)]
        );
    }

    #[test]
    fn playlist_delete_space_closes_the_gap() {
        let song = after(RegionOp::DeleteSpace, 0);
        assert_eq!(plays(&song, 2), [(0, 60), (256, 65), (1024, 67)]);
        assert_eq!(plays(&song, 1), [(512, 3)]);
    }

    #[test]
    fn playlist_duplicate_pushes_what_follows_and_copies_the_range_after_it() {
        let song = after(RegionOp::Duplicate, 0);
        assert_eq!(
            plays(&song, 2),
            [
                (0, 60),
                (256, 62),
                (512, 64),
                (768, 62),
                (1024, 64),
                (1280, 65),
                (2048, 67)
            ]
        );
        assert_eq!(plays(&song, 1), [(1536, 3)]);
    }

    #[test]
    fn playlist_copy_and_move_to_the_playhead_insert_the_range_there() {
        let song = after(RegionOp::CopyToPlayhead, 1536);
        assert_eq!(
            plays(&song, 2),
            [
                (0, 60),
                (256, 62),
                (512, 64),
                (768, 65),
                (1536, 62),
                (1792, 64),
                (2048, 67)
            ]
        );
        assert_eq!(plays(&song, 1), [(1024, 3)]);
        let song = after(RegionOp::MoveToPlayhead, 0);
        assert_eq!(
            plays(&song, 2),
            [(0, 62), (256, 64), (512, 60), (768, 65), (1536, 67)]
        );
        assert_eq!(plays(&song, 1), [(1024, 3)]);
    }

    #[test]
    fn playlist_trim_keeps_only_the_range_from_the_song_start() {
        let song = after(RegionOp::Trim, 0);
        assert_eq!(plays(&song, 2), [(0, 62), (256, 64)]);
        assert_eq!(plays(&song, 1), []);
    }

    #[test]
    fn a_clip_that_ends_on_an_edge_is_not_split() {
        let mut song = playlist();
        edit_lanes(
            &mut song.channels,
            RegionOp::InsertSpace,
            range(512, 640),
            0,
            4,
        );
        assert_eq!(song.channels[2].lanes[0].clips.len(), 3);
    }
}
