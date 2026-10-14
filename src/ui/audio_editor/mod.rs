//! The audio clip editor: an audio clip's sample drawn as a waveform under its three envelope lanes `VOL`, `PAN`,
//! `SPEED` (clip keys ±1/±2/±3, the slot's Level, Pan and Speed at value/65535), on the automation editor's frame
//! ([`crate::ui::automation::geom`]).
//!
//! - **Layout.** The frame's 1.5 U ruler row with the Snap cell, 127 fixed rows, horizontal scroll and zoom only, the
//!   scroll range 2 × the loop, a new view at 4 U per beat. The left 1.5 U column is the vertical lane list on
//!   `menuFill` ([`ItemList`], bar right).
//! - **Draw order.** 1 `background`; 2 the waveform; 3 beat marks in `controlFill` (4/2/1 px, at least 2.4 U apart);
//!   4 the other lanes' envelopes in `menuFill`; 5 on `PAN` / `SPEED` a 1 px `controlFill` centre line; 6 the past-loop
//!   overlay, black α 0.15; then the markers; 7 the current lane as the automation editor's (rows from value/65535).
//! - **Waveform** ([`columns`], [`column_span`], [`wave_rows`], [`add_column`]): clamp(0.66·`text` + 0.34·channel)
//!   at α 0.66, one 1 px column per device pixel from min/max peaks; mono centred at scale h/2, stereo halves at h/4
//!   and 3h/4 at scale h/4; a column under 1 px is a 1 px line; a reversed sample mirrored; frames per px =
//!   speed·60·zoom·rate/tempo ([`Wave`]). The columns are cached per (sample generation, frames per px, first frame).
//! - **Zoom**: (12/U)·4/d … (0.01/U)·4/d beats per point, d = `TDIV` byte 0 ([`geom::zoom_limits`]).
//! - **Markers.** The sample end, a 1 px `menuFill` line strictly inside the grid; the song loop's edges, 1 px in the
//!   channel colour.
//! - **Stretch handle** (SYNC slots only): a 1 U `text` disc with a 1 px `background` α 0.3 rim, 2 U touch box
//!   centred 1 U right of the end line and mid-grid; it grows up to 10 px while pressed and wobbles on release.
//!   Dragging sets L ([`stretch_len`]); on release, if the ratio changed, one undo step ([`Action::Stretch`]).
//! - **Ruler tap.** Hides the point selection; while playing it stops (the app's playhead rule), else it seeks only
//!   before the content end, or with a loop length of 0.
//! - **Region menu** ([`region::audio_items`]): an [`Action::Region`]. [`apply`] makes its envelope part on the current
//!   lane; the app makes that with its audio part, one edit, once the edited sample is saved (`app/audio_edit.rs`).
//! - **Gestures** on the current lane (the shared law, [`gesture::is_long`] and [`gesture::left_calm`]): a tap on empty
//!   grid deselects, else adds a point; a touch on a point (1.25 U) binds and drags it once it leaves the calm zone, a
//!   release without movement is a tap (select: handles and the floating `Copy` / `Delete`; another tap deselects); a
//!   long press (300 ms, ≤ 0.5 U) then a move is a marquee, without a move it shows `Paste` while the clipboard is set;
//!   the right-edge handle scales ≥ 2 points. Each gesture is one final edit, on release, only if something changed;
//!   the points animate (pop-in, drag growth, wobble). A move's release (as the piano roll's) leaves the moved points
//!   selected with their handle and floating row hidden, a quiet selection whose frame and handle take no touch (a tap
//!   on empty grid still only deselects it): a tap on a point then selects it with them, and a box or a paste shows a
//!   selection again.
//!
//! - The waveform spans the value area (V), so full scale meets the envelope's top and bottom rows; it is drawn once
//!   from content beat 0 (the content loop's later passes are not repeated), without the slot's Sample Start.
//! - A SYNC slot is drawn at its stretched length: the source frames over n·ratio display frames (playback stretches
//!   the same way); the source length in beats that L clamps against is the unstretched n·tempo/(60·rate), Speed left
//!   out as in the ratio's law.
//! - The stretch handle's song beat is the finger's less the handle's 1 U offset, so a press does not move the end; a
//!   drag counts once it leaves the calm zone, as every editor handle's and point's ([`gesture::left_calm`]). A re-sync
//!   sets @28 to L/speed (Speed in its law), then the clip-update rule sets the loop (@8: b·2ᵏ over the envelopes, 4·2ᵐ
//!   over @28, never below 4 or b; the visible length is left as it is).
//! - The content end of the ruler-tap rule is the loop length.
//! - The view is the scroller's ([`scroller`]): one finger scrolls the time on the ruler band only (a drag on empty
//!   grid moves nothing), two zoom it there; the wheel zooms it over the ruler and the grid, as does a trackpad pinch
//!   over the grid. The view (zoom, scroll) is kept per clip in memory, and a new one starts at beat 0; the app re-keys
//!   the views when tracks move ([`AudioEditor::rekey`]), and a view whose clip is gone goes.
//! - The other lanes draw only their curves (1.5 px), no points. The right-edge handle is a 0.6 U `text` disc 1 U
//!   right of the selection box, mid-height. A touch inside the selection's touch frame (the box grown by 1.25 U)
//!   that binds no point drags the selection.
//! - Markers are drawn between the overlay (6) and the current lane (7).
//! - The envelope clipboard is the automation editor's app-wide one ([`clipboard`]).

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use eframe::egui::{
    self, Align2, Color32, Id, Mesh, Painter, Pos2, Rangef, Rect, Response, Sense, Stroke, Ui,
    Vec2, pos2,
};
use honeydew_audio::sample::SampleData;
use honeydew_audio::sample_edit::{peaks, speed_ratio};

use crate::metrics::{FontSize, Metrics};
use crate::sample_cache::Waveform;
use crate::song::points::{self, Point, PointKey};
use crate::song::{Channel, Clip, LaneKind, PPQ, SamplerSlot, Song};
use crate::theme::channel_color;
use crate::ui::automation::geom::{self, Frame, HIT_R, View};
use crate::ui::automation::{clipboard, set_clipboard};
use crate::ui::clips::{FloatingRow, GridView, Selection, TimeAxis, mix};
use crate::ui::drum_grid::edit::marks;
use crate::ui::graph_lane::{grow, press_growth};
use crate::ui::piano_roll::notes::{auto_grid, snap_round};
use crate::ui::piano_roll::{Snap, SnapMode};
use crate::ui::popup::{self, PopupItem};
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::scroller::{self, Axis, Scroller, X};
use crate::ui::step_seq::pop_in;
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RangeRuler, RangeSelect};
use crate::ui::widgets::{self, Bar, ItemList, SnapTouch, item_list_index};
use crate::ui::{UiCtx, gesture, pinch_press, primary_down_live};

/// The lane list, top to bottom: lane i edits the clip's envelope key ±(i + 1).
pub const LANES: [&str; 3] = ["VOL", "PAN", "SPEED"];
/// Beat marks are never closer than this, in U.
pub const MARK_GAP: f32 = 2.4;
/// The waveform colour: `text` mixed this much with the channel colour's rest, at [`WAVE_ALPHA`].
pub const WAVE_TEXT: f32 = 0.66;
pub const WAVE_ALPHA: f32 = 0.66;
const PAST_LOOP_ALPHA: f32 = 0.15;
/// Points (U) and curve lines (device px).
const POINT_R: f32 = 0.2;
const LINE_PX: f32 = 1.5;
/// The `Paste` button's side, U.
const PASTE_U: f32 = 1.5;
/// The right-edge handle: diameter, and its offset right of the box, U.
const SCALE_D: f32 = 0.6;
const SCALE_OFF: f32 = 1.0;
/// The stretch handle: disc and touch box (U), its offset right of the end line (U).
const STRETCH_D: f32 = 1.0;
const STRETCH_BOX: f32 = 2.0;
const STRETCH_OFF: f32 = 1.0;
/// L is clamped to these multiples of the source length.
pub(crate) const STRETCH: [f64; 2] = [0.2, 10.0];
/// The point animation: a ← 0.6·a per frame; the pop-in law; the drag floor.
const ANIM_RATE: f32 = 0.6;
const POP_GAIN: f32 = 1.0515;
const POP_W: f32 = 1.885;
const DRAG_FLOOR: f32 = 0.01;
/// The floating buttons over a point selection, as over automation points.
const OPS: [&str; 2] = ["Copy", "Delete"];

/// A clip as the editor addresses it: (channel, lane, clip id).
pub type At = (usize, usize, u32);

/// The transport as the editor sees it this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Transport {
    /// The transport's song tick.
    pub tick: u64,
    /// How far past `tick` the playhead is drawn ([`crate::audio_link::AudioLink::playhead_lead`]): the line and the
    /// disc only, never an edit.
    pub lead: u64,
    pub playing: bool,
    /// REC is armed: the playhead disc shows the record icon.
    pub armed: bool,
    /// Preview plays the shown clip.
    pub previewing: bool,
    /// The pointer is over an overlay: the wheel and zoom are not the editor's.
    pub covered: bool,
}

/// What the editor did this frame; [`apply`] makes the song edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A gesture's final points for one envelope (one undo step).
    Points { key: PointKey, points: Vec<Point> },
    /// The stretch handle's release: the length L in beats and the STR1 ratio.
    Stretch { len_beats: f64, ratio: f64 },
    /// The ruler's range, as the song's play loop.
    PlayLoop(Option<(u32, u32)>),
    /// A ruler tap: the song tick to seek to (the app stops instead while playing).
    SetPlayhead(u64),
    /// A tap on the playhead knob: the transport plays.
    Play,
    /// The playhead knob's drop: seek to this song tick, playing or stopped, as the playlist's knob does.
    Seek(u64),
    /// A region menu pick for the current lane `key`: content ticks, the playhead at content tick `playhead`.
    Region {
        key: PointKey,
        op: RegionOp,
        range: TimeRange,
        playhead: u32,
    },
}

/// Where a sample sits on the content time axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    /// Source frames.
    pub frames: usize,
    /// Source frames a content beat: speed·60·rate/tempo, over the stretch ratio of a stretched slot.
    pub per_beat: f64,
    pub reversed: bool,
}

impl Wave {
    /// `frames` at `rate` Hz and `tempo` BPM, played at the Speed param `speed` and stretched by `stretch`.
    pub fn new(
        frames: usize,
        rate: u32,
        tempo: f64,
        speed: f32,
        stretch: f64,
        reversed: bool,
    ) -> Self {
        Self {
            frames,
            per_beat: speed_ratio(speed) * 60.0 * f64::from(rate) / (tempo * stretch),
            reversed,
        }
    }

    /// The sample's end, in content beats.
    pub fn end(self) -> f64 {
        self.frames as f64 / self.per_beat
    }
}

/// The stretch a slot's sample plays and draws with: STR1's ratio while stretch (SYNC) is on, else 1 (also for a
/// ratio a file holds that is no length multiplier).
pub fn stretch_of(slot: &SamplerSlot) -> f64 {
    let r = slot.file.stretch_ratio;
    if slot.file.stretch_mode != 0 && r.is_finite() && r > 0.0 {
        r
    } else {
        1.0
    }
}

/// One channel's (min, max) per device-pixel column: `width` columns from content beat `left`, `beats_per_px`
/// apart ([`peaks`], reading the channel's `summary` where a column covers whole blocks); a reversed sample's are the
/// mirrored range's, in reverse.
pub fn columns(
    samples: &[f32],
    summary: &[(f32, f32)],
    wave: Wave,
    left: f64,
    beats_per_px: f64,
    width: usize,
) -> Vec<(f32, f32)> {
    let fpp = wave.per_beat * beats_per_px;
    let first = left * wave.per_beat;
    if !wave.reversed {
        return peaks(samples, summary, fpp, first, width);
    }
    let mirrored = wave.frames as f64 - first - width as f64 * fpp;
    let mut cols = peaks(samples, summary, fpp, mirrored, width);
    cols.reverse();
    cols
}

/// A column's y span for peaks `lo`…`hi` about `centre` at `scale` (y grows down), on whole `px`; under one `px` it
/// is a `px` line at the span's middle.
pub fn column_span(lo: f32, hi: f32, centre: f32, scale: f32, px: f32) -> Rangef {
    let (top, bottom) = (centre - hi * scale, centre - lo * scale);
    if bottom - top < px {
        let y = ((top + bottom) / 2.0 / px).floor() * px;
        return Rangef::new(y, y + px);
    }
    Rangef::new((top / px).round() * px, (bottom / px).round() * px)
}

/// Adds the waveform column at `x` to `mesh`: `px` wide over peaks `lo`…`hi` about `centre` at `scale`
/// ([`column_span`]), in `ink`. Every waveform paints its columns as one such mesh of exact quads, never with
/// `rect_filled`: egui draws a 1 px wide rect as a line segment, which shrinks a 1 px tall column (the law's "1 px
/// line") to nothing.
pub fn add_column(
    mesh: &mut Mesh,
    x: f32,
    (lo, hi): (f32, f32),
    (centre, scale): (f32, f32),
    px: f32,
    ink: Color32,
) {
    let y = column_span(lo, hi, centre, scale, px);
    mesh.add_colored_rect(Rect::from_x_y_ranges(x..=x + px, y), ink);
}

/// The quads of the untextured meshes in `shapes` (a waveform's columns, [`add_column`]), with their colours, in
/// order.
#[cfg(test)]
pub(crate) fn mesh_quads<'a>(
    shapes: impl IntoIterator<Item = &'a egui::Shape>,
) -> Vec<(Rect, Color32)> {
    let meshes = shapes.into_iter().filter_map(|s| match s {
        egui::Shape::Mesh(m) if m.texture_id == egui::TextureId::default() => Some(m),
        _ => None,
    });
    let quad = |q: &[egui::epaint::Vertex]| (Rect::from_two_pos(q[0].pos, q[3].pos), q[0].color);
    meshes
        .flat_map(|m| m.vertices.chunks_exact(4).map(quad))
        .collect()
}

/// Each channel's centre and scale in `area`: mono centred at h/2; stereo halves at h/4 and 3h/4, at h/4.
pub fn wave_rows(area: Rect, stereo: bool) -> Vec<(f32, f32)> {
    let (top, h) = (area.top(), area.height());
    if stereo {
        vec![(top + h / 4.0, h / 4.0), (top + 3.0 * h / 4.0, h / 4.0)]
    } else {
        vec![(top + h / 2.0, h / 2.0)]
    }
}

/// The waveform's device-pixel columns over `grid`: the first one's x and how many.
pub fn pixel_columns(grid: Rect, m: &Metrics) -> (f32, usize) {
    let x0 = m.round_px(grid.left());
    (
        x0,
        ((grid.right() - x0) * m.ppp()).floor().max(0.0) as usize,
    )
}

/// The stretch handle's touch box for the end line at `end_x`: 2 U, centred 1 U right of it and mid-grid.
pub fn stretch_box(f: &Frame, end_x: f32, m: &Metrics) -> Rect {
    let centre = pos2(end_x + m.u(STRETCH_OFF), f.grid.center().y);
    Rect::from_center_size(centre, Vec2::splat(m.u(STRETCH_BOX)))
}

/// L for the end line dragged to content tick `tick`: the snapped song beat less the clip origin (`origin`, a song
/// tick), clamped to 0.2…10 × `src_beats`; in beats.
pub fn stretch_len(tick: f64, origin: i64, snap: Option<f64>, src_beats: f64) -> f64 {
    let o = origin as f64;
    let len = (snap_round(tick + o, snap) - o) / f64::from(PPQ);
    len.clamp(STRETCH[0] * src_beats, STRETCH[1] * src_beats)
}

/// The STR1 ratio for L: trunc(rate·60·L/tempo)/source frames.
pub fn stretch_ratio(len: f64, rate: u32, tempo: f64, frames: usize) -> f64 {
    (f64::from(rate) * 60.0 * len / tempo).trunc() / frames as f64
}

fn find(song: &Song, at: At) -> Option<(&Channel, &Clip)> {
    let ch = song.channels.get(at.0)?;
    let lane = ch.lanes.get(at.1).filter(|l| l.kind == LaneKind::Audio)?;
    Some((ch, lane.clips.iter().find(|c| c.id == at.2)?))
}

fn find_mut(song: &mut Song, at: At) -> Option<&mut Clip> {
    let lane =
        (song.channels.get_mut(at.0)?.lanes.get_mut(at.1)).filter(|l| l.kind == LaneKind::Audio)?;
    lane.clips.iter_mut().find(|c| c.id == at.2)
}

/// The envelope rows of a region op (as the automation editor's; `Insert silence` is `Insert space`, `Clear` is
/// `Delete`); `Normalize` and the fades are audio only.
fn envelope_op(pts: &mut Vec<Point>, op: RegionOp, r: TimeRange, playhead: u32, loop_end: u32) {
    let (s, e) = (r.start, r.end);
    match op {
        RegionOp::InsertSilence | RegionOp::InsertSpace => points::insert_space(pts, s, e),
        RegionOp::Duplicate => points::duplicate(pts, s, e),
        RegionOp::CopyToPlayhead | RegionOp::MoveToPlayhead => {
            points::to_playhead(pts, s, e, playhead, op == RegionOp::MoveToPlayhead);
        }
        RegionOp::Clear | RegionOp::Delete => points::delete(pts, s, e),
        RegionOp::DeleteSpace => points::delete_space(pts, s, e),
        RegionOp::Trim => points::trim(pts, s, e, loop_end),
        _ => {}
    }
}

/// Makes `action`'s edit of the clip `at` in `song`: whether the song changed (then it is one undo step). A region op
/// makes its envelope part only (its audio part is the app's) and clears the play loop (all but `Duplicate`).
pub fn apply(song: &mut Song, at: At, action: &Action) -> bool {
    match *action {
        Action::PlayLoop(play_loop) => song.play_loop = play_loop,
        Action::Region { op, .. } if !op.keeps_loop() => song.play_loop = None,
        _ => {}
    }
    let b = song.time_sig.bar_beats();
    let Some(clip) = find_mut(song, at) else {
        return false;
    };
    let set = |clip: &mut Clip, key, pts: Vec<Point>| {
        let changed = points::points(clip, key) != pts;
        if changed {
            points::set_points(clip, key, pts);
            clip.fit_loop(b);
        }
        changed
    };
    match action {
        Action::Points { key, points } => set(clip, *key, points.clone()),
        &Action::Region {
            key,
            op,
            range,
            playhead,
        } => {
            let mut pts = points::points(clip, key);
            let loop_end = clip.loop_ticks().map_or(0, |t| t as u32);
            envelope_op(&mut pts, op, range, playhead, loop_end);
            set(clip, key, pts)
        }
        &Action::Stretch { len_beats, ratio } => {
            let Some(slot) = clip.file.sample.as_deref_mut() else {
                return false;
            };
            let speed = speed_ratio(slot.params[2]);
            let old_ratio = std::mem::replace(&mut slot.file.stretch_ratio, ratio);
            let lengths = |c: &Clip| {
                (
                    c.file.source_len_beats,
                    c.loop_len_beats,
                    c.visible_len_beats,
                )
            };
            let before = lengths(clip);
            // @28 = BPM·frames/(rate·60·speed) with the stretched frames: L at Speed 1, L/speed otherwise; then the
            // @8 rule (b·2ᵏ over the envelopes, 4·2ᵐ over @28); the visible length is left as it is.
            clip.file.source_len_beats = len_beats / speed;
            clip.fit_loop(b);
            old_ratio != ratio || lengths(clip) != before
        }
        Action::PlayLoop(_) | Action::SetPlayhead(_) | Action::Play | Action::Seek(_) => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Anim {
    Pop,
    Grow,
    Wobble,
}

/// Points being moved (or scaled, from the right-edge handle), from `orig` as they were at the press.
#[derive(Debug)]
struct Move {
    at: Pos2,
    scale: bool,
    orig: Vec<Point>,
    sel: Vec<bool>,
    /// The point the touch bound, and whether it was selected before.
    bound: Option<(usize, bool)>,
    /// The finger has left the calm zone ([`gesture::left_calm`]).
    armed: bool,
    preview: Option<(Vec<Point>, Vec<bool>)>,
}

#[derive(Debug, Default)]
enum Gesture {
    #[default]
    Idle,
    /// On empty grid: a tap, a long press or a scroll.
    Press {
        at: Pos2,
        since: f64,
    },
    Pan {
        last: Pos2,
    },
    /// A long press that has not moved: `Paste` on release.
    Hold {
        at: Pos2,
    },
    Marquee {
        from: Pos2,
        to: Pos2,
    },
    Points(Move),
}

/// What a waveform's columns were made for: the sample (its generation: a `Weak`, so a replaced sample's frames are
/// freed; it pins only the small `SampleData` block, so no new sample takes its address), the wave, the left beat and
/// the beats a column (as bits), and how many.
type PeakKey = (Weak<SampleData>, Wave, u64, u64, usize);

/// A gesture's final points, their selection and the animation it starts.
type Edit = (Vec<Point>, Vec<bool>, Anim);

/// The waveform columns of the last view (one `Vec` per channel), and the key they were made for. A scroll or zoom
/// makes them again from the sample's summary ([`Waveform`]), at most 2·256 frames a column.
#[derive(Default)]
struct Peaks {
    key: Option<PeakKey>,
    cols: Vec<Vec<(f32, f32)>>,
    /// Recomputations (tests).
    misses: usize,
}

impl Peaks {
    /// The columns of `sample` for `wave`, `width` of them from beat `left`, `bpp` beats a column: cached while the
    /// sample's frames (its generation) and the view stay.
    fn get(
        &mut self,
        source: &Waveform,
        wave: Wave,
        left: f64,
        bpp: f64,
        width: usize,
    ) -> &[Vec<(f32, f32)>] {
        let sample = &source.data;
        let hit = (self.key.as_ref()).is_some_and(|(s, w, l, b, n)| {
            std::ptr::eq(s.as_ptr(), Arc::as_ptr(sample))
                && *w == wave
                && *l == left.to_bits()
                && *b == bpp.to_bits()
                && *n == width
        });
        if !hit {
            self.misses += 1;
            self.cols = (source.channels())
                .map(|(c, summary)| columns(c, summary, wave, left, bpp, width))
                .collect();
            let key = (
                Arc::downgrade(sample),
                wave,
                left.to_bits(),
                bpp.to_bits(),
                width,
            );
            self.key = Some(key);
        }
        &self.cols
    }
}

/// The stretch handle's gesture: the length dragged to, and the press growth and release wobble.
#[derive(Default)]
struct Stretch {
    len: Option<f64>,
    origin: Option<Pos2>,
    pressed: Option<f64>,
    released: Option<(f64, f32)>,
}

/// The audio clip editor's state (module docs).
#[derive(Default)]
pub struct AudioEditor {
    /// The current envelope lane, an index of [`LANES`].
    pub lane: usize,
    pub snap: Snap,
    /// Each clip's view, by (channel, lane, clip id).
    pub(crate) views: HashMap<At, View>,
    /// One flag per point of the current lane, and the clip, lane and points they belong to: anything else is an
    /// outside change (another clip, an undo), which drops the selection and any gesture.
    sel: Vec<bool>,
    /// The selection is quiet: kept, its handle and floating row hidden, as a move's release leaves it (module docs).
    quiet: bool,
    seen: Option<(At, usize, Vec<Point>)>,
    gesture: Gesture,
    anim: Option<(Anim, f64)>,
    /// The `Paste` button and the tick it pastes at.
    paste: Option<(Rect, f64)>,
    row: FloatingRow,
    region: RangeSelect,
    /// The playhead knob on the ruler.
    knob: PlayheadKnob,
    snap_menu: bool,
    peaks: Peaks,
    stretch: Stretch,
    /// The stretch handle's touch box as last drawn.
    handle: Option<Rect>,
    /// The view's scroll and zoom controller.
    scroller: Scroller,
}

impl AudioEditor {
    /// Follows every clip's view and the selection's clip to `track(channel, lane)` after channels or tracks moved;
    /// what was on a gone track goes (as [`crate::ui::automation::AutomationEditor::rekey`]).
    pub(crate) fn rekey(&mut self, track: &dyn Fn(usize, usize) -> Option<(usize, usize)>) {
        let views = std::mem::take(&mut self.views).into_iter();
        let moved = |((c, l, id), v)| track(c, l).map(|(c, l)| ((c, l, id), v));
        self.views = views.filter_map(moved).collect();
        self.seen = (self.seen.take()).and_then(|((c, l, id), lane, pts)| {
            let (c, l) = track(c, l)?;
            Some(((c, l, id), lane, pts))
        });
    }

    /// The editor stops showing with a touch it will not see end (`app::left_view`): the touch ends as a pinch's
    /// does, changing nothing; a drag's preview (the points' or the stretch handle's length) goes with it.
    pub(crate) fn end_touch(&mut self) {
        self.gesture = Gesture::Idle;
        (self.stretch.len, self.stretch.origin, self.stretch.pressed) = (None, None, None);
    }

    /// Draws the editor over `ui`'s rect for the audio clip `at` of `song`, its sample `sample` if decoded, and
    /// returns what it did. Nothing without such a clip.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        song: &Song,
        at: At,
        source: Option<&Waveform>,
        t: Transport,
    ) -> Vec<Action> {
        let sample = source.map(|w| &w.data);
        let mut out = Vec::new();
        self.handle = None;
        let Some((ch, clip)) = find(song, at) else {
            return out;
        };
        let (m, c) = (cx.m, &cx.theme.colors);
        let (u, px) = (m.u(1.0), 1.0 / m.ppp());
        let rect = ui.max_rect();
        let f = Frame::new(rect, m);
        let sig = song.time_sig;
        let now = ui.input(|i| i.time);
        let id = Id::new("audio_editor");
        let color = channel_color(ch.hue, cx.theme);
        let ink = color.to_color32();

        // 1: the background (the ruler's rect too), and the lane list on `menuFill`.
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, c.background.to_color32());
        painter.rect_filled(f.column, 0.0, c.menu_fill.to_color32());
        let mut lane = self.lane.min(LANES.len() - 1);
        let list = ItemList::new(id.with("lanes"), f.column, &LANES, Bar::Right, ink);
        item_list_index(ui, cx, &list, &mut lane);
        self.lane = lane;

        // The selection follows the clip, the lane and their points.
        let key = PointKey::Envelope(self.lane as i16);
        let max = key.max();
        let mut pts = points::points(clip, key);
        if (self.seen.as_ref()).is_none_or(|s| s.0 != at || s.1 != self.lane || s.2 != pts) {
            if self.seen.as_ref().is_some_and(|s| s.0 != at) {
                // Another clip's view: a fling of the last one stops.
                self.scroller = Scroller::default();
            }
            self.sel = vec![false; pts.len()];
            self.gesture = Gesture::Idle;
            self.paste = None;
            self.seen = Some((at, self.lane, pts.clone()));
        }

        // The clip's view, by the scroller (no y): one finger scrolls the time on the ruler band only, two zoom it
        // there; the wheel zooms it over the ruler and the grid, where a trackpad pinch zooms it too.
        // Views of gone clips go.
        (self.views).retain(|&clip, _| find(song, clip).is_some());
        let view = (self.views.get(&at).copied()).unwrap_or_else(|| View::initial(None, u));
        let loop_beats = clip.loop_len_beats;
        let mut axes = [geom::axis(view, u, sig, loop_beats, f.grid), Axis::OFF];
        let was = axes[X];
        let options = scroller::Options {
            covered: t.covered,
            follow: false,
        };
        let zones = geom::zones(&f);
        let report = (self.scroller).update(ui, id.with("scroll"), u, &zones, &mut axes, options);
        let view = geom::take(view, was, axes[X]);
        let axis = view.axis(f.grid.left());
        let snap = self.snap.active(auto_grid(view.beat_w, u, sig), sig);

        // The gestures: the grid's first, then the widgets over it.
        let grid = ui.interact(f.grid, id.with("grid"), Sense::click_and_drag());
        crate::ui::focus::register(ui, grid.id, grid.rect, crate::ui::focus::Kind::Container);
        let edit = self.grid_input(ui, &grid, &f, axis, &pts, max, snap, m, now);
        self.views.insert(at, view);
        if let Some((new, sel, anim)) = edit {
            pts = self.commit(at, key, new, sel, anim, now, &mut out);
        }
        let origin = region::clip_origin(Some(clip));
        let slot = clip.file.sample.as_deref();
        let tempo = song.tempo_bpm;
        let wave_of = |stretch: f64| {
            let (slot, s) = slot.zip(sample)?;
            let w = Wave::new(
                s.frames(),
                s.sample_rate,
                tempo,
                slot.params[2],
                stretch,
                slot.file.reverse,
            );
            (w.per_beat > 0.0 && w.per_beat.is_finite()).then_some(w)
        };
        let synced = slot.is_some_and(|s| s.file.stretch_mode != 0);
        if let Some((slot, s)) = slot.zip(sample).filter(|_| synced)
            && let Some(w) = wave_of(self.preview_stretch(s, tempo, slot))
        {
            let end_x = axis.x(w.end() * f64::from(PPQ));
            let src = s.frames() as f64 * tempo / (60.0 * f64::from(s.sample_rate));
            let ctx = (&f, axis, origin, snap, src, end_x);
            if let Some(stretch) = self.stretch_input(ui, id, ctx, m, now) {
                let ratio = stretch_ratio(stretch, s.sample_rate, tempo, s.frames());
                if ratio != stretch_of(slot) {
                    out.push(Action::Stretch {
                        len_beats: stretch,
                        ratio,
                    });
                }
            }
        }
        let wave = slot.and_then(|slot| {
            let s = sample?;
            wave_of(self.preview_stretch(s, tempo, slot))
        });

        // 2 … 7 over the grid.
        let gp = ui.painter_at(f.grid);
        if let (Some(s), Some(w)) = (source, wave) {
            let tint = mix(c.text, color, 1.0 - WAVE_TEXT).to_color32();
            self.paint_wave(&gp, &f, axis, s, w, tint.gamma_multiply(WAVE_ALPHA), m);
        }
        let (t0, t1) = (axis.tick(f.grid.left()), axis.tick(f.grid.right()));
        let control = c.control_fill.to_color32();
        for (tick, w) in marks(t0.max(0.0), t1, axis.beat_w, u, sig, MARK_GAP) {
            let x = m.floor_px(axis.x(tick)) - (w / 2.0).floor() * px;
            let r = Rect::from_x_y_ranges(x..=x + w * px, f.grid.y_range());
            gp.rect_filled(r, 0.0, control);
        }
        let line_w = LINE_PX * px;
        let menu = c.menu_fill.to_color32();
        for other in (0..LANES.len()).filter(|&k| k != self.lane) {
            let key = PointKey::Envelope(other as i16);
            curve(
                &gp,
                &f,
                axis,
                &points::points(clip, key),
                key.max(),
                line_w,
                |_| menu,
            );
        }
        if self.lane > 0 {
            let y = m.floor_px(f.value.center().y);
            let r = Rect::from_x_y_ranges(f.grid.x_range(), y..=y + px);
            gp.rect_filled(r, 0.0, control);
        }
        let loop_ticks = loop_beats * f64::from(PPQ);
        let past = axis.x(loop_ticks).max(f.grid.left());
        if past < f.grid.right() {
            let r = Rect::from_x_y_ranges(past..=f.grid.right(), f.value.y_range());
            let alpha = (PAST_LOOP_ALPHA * 255.0).round() as u8;
            gp.rect_filled(r, 0.0, Color32::from_black_alpha(alpha));
        }
        let marker = |x: f32, fill: Color32| {
            if f.grid.left() < x && x < f.grid.right() {
                let x = m.floor_px(x);
                gp.rect_filled(
                    Rect::from_x_y_ranges(x..=x + px, f.grid.y_range()),
                    0.0,
                    fill,
                );
            }
        };
        if let Some(w) = wave {
            marker(axis.x(w.end() * f64::from(PPQ)), menu);
        }
        if let Some((s, e)) = song.play_loop {
            for edge in [s, e] {
                marker(axis.x((i64::from(edge) - origin) as f64), ink);
            }
        }
        let shown = match &self.gesture {
            Gesture::Points(Move {
                preview: Some((p, s)),
                ..
            }) => (p.clone(), s.clone()),
            _ => (pts.clone(), self.sel.clone()),
        };
        self.paint_points(&gp, cx, &f, axis, &shown, max, ink, now);
        let boxing = if let Gesture::Marquee { from, to } = self.gesture {
            let r = Rect::from_two_pos(from, to);
            let text = c.text.to_color32();
            gp.rect(
                r,
                0.0,
                text.gamma_multiply(0.1),
                Stroke::new(px, text),
                egui::StrokeKind::Inside,
            );
            true
        } else {
            false
        };
        if let Some(w) = wave.filter(|_| synced) {
            self.paint_stretch(ui, cx, &f, axis.x(w.end() * f64::from(PPQ)), now);
        }

        // The ruler, its range and region menu, and the playhead.
        let rp = ui.painter_at(f.time);
        timeline::ruler(&rp, f.time, f.time, axis.origin, axis.beat_w, sig, cx);
        let playhead = region::playhead_tick(Some(clip), t.tick, t.previewing)
            .map(|p| geom::playhead(p, loop_ticks));
        let ruler = RangeRuler {
            strip: f.time,
            axis,
            grid: snap,
            sig,
            ink,
            range: TimeRange::in_clip(song.play_loop, origin),
            playhead,
        };
        if report.scrolled[X] || report.zoomed[X] {
            self.region.view_moved();
        }
        let r = self
            .region
            .show_with(ui, id.with("region"), &ruler, &region::audio_items, cx);
        // A ruler press waiting for its long press is the scroller's too, until its range starts.
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        if let Some(range) = r.range {
            out.push(Action::PlayLoop(range.and_then(|r| r.song_loop(origin))));
        }
        if let Some(tick) = r.playhead {
            self.sel.fill(false);
            self.paste = None;
            if t.playing || f64::from(tick) < loop_ticks || loop_ticks <= 0.0 {
                out.push(Action::SetPlayhead(region::seek_tick(
                    Some(clip),
                    tick,
                    t.previewing,
                )));
            }
        }
        if let Some((op, range)) = r.pick {
            out.push(Action::Region {
                key,
                op,
                range,
                playhead: playhead.map_or(0, |p| p as u32),
            });
        }
        // Clipped at the value column's right edge: at beat 0 the line's left half shows in the t gap. While the knob
        // is dragged, the line and the disc follow the finger.
        let line = ui.painter_at(f.grid.with_min_x(f.column.right()));
        let drawn = region::playhead_tick(Some(clip), t.tick + t.lead, t.previewing)
            .map(|p| geom::playhead(p, loop_ticks));
        let x = (self.knob).line_x(
            ui,
            m,
            &ruler,
            loop_ticks - 1.0,
            axis.x(drawn.unwrap_or(0.0)),
        );
        timeline::playhead_line(&line, f.grid, x, cx);

        // Over the grid: the floating row, `Paste`, the Snap button and its menu.
        self.row.covered = t.covered;
        if let Some((new, sel)) = self.floating_row(ui, cx, &f, axis, &pts, max, boxing) {
            self.commit(at, key, new, sel, Anim::Wobble, now, &mut out);
        }
        if let Some((new, sel)) = self.paste_button(ui, cx, &pts, max, snap) {
            self.quiet = false;
            self.commit(at, key, new, sel, Anim::Pop, now, &mut out);
        }
        self.snap_button(ui, id, f.snap, cx);
        // The playhead knob over them all, the Snap button too, which it overhangs at beat 0 (the playhead line's
        // z-order), and whose touches it takes there while it shows, as the playlist's.
        self.knob.armed = t.armed;
        let shown = timeline::knob_shown(t.playing, self.row.showing());
        match (self.knob).show(ui, id.with("knob"), f.time, x, shown, cx) {
            Some(KnobGesture::Tap) => out.push(Action::Play),
            Some(KnobGesture::Drop(x)) => {
                let tick = self.knob.drop_tick(ui, &ruler, loop_ticks - 1.0, x) as u32;
                let tick = region::seek_tick(Some(clip), tick, t.previewing);
                out.push(Action::Seek(tick));
            }
            None => {}
        }
        self.snap_menu(ui, id, f.snap, cx);
        if self.anim.is_some() {
            ui.ctx().request_repaint();
        }
        out
    }

    /// A gesture's final points: they and their selection become the editor's, and the edit goes out.
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &mut self,
        at: At,
        key: PointKey,
        pts: Vec<Point>,
        sel: Vec<bool>,
        anim: Anim,
        now: f64,
        out: &mut Vec<Action>,
    ) -> Vec<Point> {
        self.sel = sel;
        self.seen = Some((at, self.lane, pts.clone()));
        self.anim = Some((anim, now));
        out.push(Action::Points {
            key,
            points: pts.clone(),
        });
        pts
    }

    /// The stretch the sample shows with: the dragged length's while the handle is dragged, else the slot's.
    fn preview_stretch(&self, s: &SampleData, tempo: f64, slot: &SamplerSlot) -> f64 {
        match self.stretch.len {
            Some(len) => {
                stretch_ratio(len, s.sample_rate, tempo, s.frames()).max(f64::MIN_POSITIVE)
            }
            None => stretch_of(slot),
        }
    }

    /// The selected points' box: x at their ticks, y at their drawn centres.
    fn sel_box(
        &self,
        f: &Frame,
        axis: TimeAxis,
        pts: &[Point],
        sel: &[bool],
        max: u16,
    ) -> Option<Rect> {
        let at = |p: &Point| pos2(axis.x(p.tick.into()), f.point_y(p.value, max));
        (pts.iter().zip(sel))
            .filter(|(_, s)| **s)
            .map(|(p, _)| Rect::from_min_max(at(p), at(p)))
            .reduce(Rect::union)
    }

    /// The right-edge handle, with ≥ 2 points selected.
    fn scale_handle(&self, bx: Option<Rect>, sel: &[bool], m: &Metrics) -> Option<Rect> {
        let bx = bx.filter(|_| sel.iter().filter(|s| **s).count() >= 2)?;
        let centre = pos2(bx.right() + m.u(SCALE_OFF), bx.center().y);
        Some(Rect::from_center_size(centre, Vec2::splat(m.u(SCALE_D))))
    }

    /// The gesture a press at `p` starts (module docs).
    #[allow(clippy::too_many_arguments)]
    fn press(
        &mut self,
        f: &Frame,
        axis: TimeAxis,
        pts: &[Point],
        max: u16,
        p: Pos2,
        m: &Metrics,
        now: f64,
    ) -> Gesture {
        let bx = self.sel_box(f, axis, pts, &self.sel, max);
        let moving = |scale, bound, sel: Vec<bool>| {
            Gesture::Points(Move {
                at: p,
                scale,
                orig: pts.to_vec(),
                sel,
                bound,
                armed: false,
                preview: None,
            })
        };
        // A quiet selection's handle and frame take no touch; a touch on one of its points binds it as not showing.
        if self
            .scale_handle(bx, &self.sel, m)
            .is_some_and(|h| !self.quiet && h.center().distance(p) <= h.width())
        {
            return moving(true, None, self.sel.clone());
        }
        if let Some(i) = f.hit(axis, pts, max, p, m) {
            let was = self.sel[i] && !self.quiet;
            if !self.sel[i] {
                self.sel = (0..pts.len()).map(|j| j == i).collect();
            }
            self.anim = Some((Anim::Grow, now));
            return moving(false, Some((i, was)), self.sel.clone());
        }
        if !self.quiet && bx.is_some_and(|r| r.expand(m.u(HIT_R)).contains(p)) {
            self.anim = Some((Anim::Grow, now));
            return moving(false, None, self.sel.clone());
        }
        Gesture::Press { at: p, since: now }
    }

    /// Taps, long presses and drags on the grid (module docs). Returns a gesture's final points, selection and
    /// animation, and the scroll in points.
    #[allow(clippy::too_many_arguments)]
    fn grid_input(
        &mut self,
        ui: &Ui,
        grid: &Response,
        f: &Frame,
        axis: TimeAxis,
        pts: &[Point],
        max: u16,
        snap: Option<f64>,
        m: &Metrics,
        now: f64,
    ) -> Option<Edit> {
        let down = primary_down_live(ui, grid.id);
        let (pressed, pos, multi) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.primary_pressed(),
                p.interact_pos(),
                i.multi_touch().is_some(),
            )
        });
        // A pinch only zooms; a press held over a focus loss is up and stale (`primary_down_live`): either ends the
        // gesture with no tap, drop or `Paste`.
        let stale = !down && ui.input(|i| i.pointer.primary_down());
        if multi || stale {
            self.gesture = Gesture::Idle;
            return None;
        }
        self.quiet &= self.sel.contains(&true);
        if pressed
            && grid.is_pointer_button_down_on()
            && let Some(p) = pos
        {
            self.paste = None;
            self.gesture = self.press(f, axis, pts, max, p, m, now);
        }
        if !matches!(self.gesture, Gesture::Idle) {
            ui.ctx().request_repaint();
        }
        let u = m.u(1.0);
        if down && let Some(p) = pos {
            let next = match &mut self.gesture {
                Gesture::Press { at, since } => {
                    let moved = gesture::left_calm(*at, Some(p), u);
                    if gesture::is_long(now - *since) {
                        Some(if moved {
                            Gesture::Marquee { from: *at, to: p }
                        } else {
                            Gesture::Hold { at: *at }
                        })
                    } else if moved {
                        // A drag: nothing scrolls here (the touch x zone is the ruler band).
                        Some(Gesture::Pan { last: p })
                    } else {
                        None
                    }
                }
                Gesture::Hold { at } if gesture::left_calm(*at, Some(p), u) => {
                    Some(Gesture::Marquee { from: *at, to: p })
                }
                Gesture::Marquee { to, .. } => {
                    *to = p;
                    None
                }
                Gesture::Pan { last } => {
                    *last = p;
                    None
                }
                Gesture::Points(g) => {
                    g.armed |= gesture::left_calm(g.at, Some(p), u);
                    if g.armed {
                        let d = p - g.at;
                        let dt = f64::from(d.x / axis.beat_w) * f64::from(PPQ);
                        let dr = f64::from(d.y) * 127.0 / f64::from(f.value.height());
                        g.preview = if g.scale {
                            points::scaled(&g.orig, &g.sel, dt, snap)
                        } else {
                            points::moved(&g.orig, &g.sel, dt, dr, snap, max)
                        };
                    }
                    None
                }
                _ => None,
            };
            if let Some(g) = next {
                self.gesture = g;
            }
        }
        if down {
            return None;
        }
        let gesture = match std::mem::take(&mut self.gesture) {
            // Released in the frame its 300 ms come due: a long press without a move.
            Gesture::Press { at, since } if gesture::is_long(now - since) => Gesture::Hold { at },
            g => g,
        };
        match gesture {
            // A quiet selection too.
            Gesture::Press { .. } if self.sel.contains(&true) => {
                self.sel.fill(false);
                None
            }
            Gesture::Press { at, .. } => {
                let (new, sel) = points::create(pts, axis.tick(at.x), f.row_at(at.y), snap, max);
                Some((new, sel, Anim::Pop))
            }
            Gesture::Hold { at } => {
                if !clipboard(ui.ctx()).is_empty() {
                    let r = f.row_at(at.y).floor().clamp(0.0, points::LAST_ROW);
                    let centre = pos2(at.x, f.row_y(r + 0.5));
                    let button = Rect::from_center_size(centre, Vec2::splat(m.u(PASTE_U)));
                    self.paste = Some((button, axis.tick(at.x)));
                }
                None
            }
            Gesture::Marquee { from, to } => {
                let ticks = [axis.tick(from.x), axis.tick(to.x)];
                self.sel = points::marquee(pts, ticks, [f.row_at(from.y), f.row_at(to.y)], max);
                self.quiet = false;
                None
            }
            Gesture::Points(g) => {
                self.anim = Some((Anim::Wobble, now));
                // A move's release leaves its selection quiet; a tap or a scale shows it.
                self.quiet = g.armed && !g.scale;
                match (g.preview, g.armed, g.bound) {
                    (Some((new, sel)), ..) => Some((new, sel, Anim::Wobble)),
                    // A tap: a bound point not showing selected is the selection alone (out of a quiet one too);
                    // another tap deselects.
                    (None, false, Some((i, false))) => {
                        self.sel = (0..pts.len()).map(|j| j == i).collect();
                        None
                    }
                    (None, false, _) => {
                        self.sel.fill(false);
                        None
                    }
                    // Dragged back where it started.
                    (None, true, _) => {
                        self.sel = g.sel;
                        None
                    }
                }
            }
            Gesture::Pan { .. } | Gesture::Idle => None,
        }
    }

    /// The stretch handle's input: a drag sets L; returns L on release (module docs).
    fn stretch_input(
        &mut self,
        ui: &mut Ui,
        id: Id,
        (f, axis, origin, snap, src, end_x): (&Frame, TimeAxis, i64, Option<f64>, f64, f32),
        m: &Metrics,
        now: f64,
    ) -> Option<f64> {
        let touch = stretch_box(f, end_x, m);
        // Off the grid it takes no new press; a drag that takes it there carries on.
        if touch.intersects(f.grid) {
            self.handle = Some(touch);
        } else if self.stretch.pressed.is_none() {
            return None;
        }
        let response = ui.interact(touch, id.with("stretch"), Sense::click_and_drag());
        // Every frame, for their latches: a press held over a focus loss is up, and a second finger is a pinch's.
        let down = primary_down_live(ui, response.id);
        let pinch = pinch_press(ui, response.id);
        let (pressed, pos, held) = ui.input(|i| {
            let p = &i.pointer;
            (p.primary_pressed(), p.interact_pos(), p.primary_down())
        });
        if pressed && response.is_pointer_button_down_on() {
            self.stretch.pressed = Some(now);
            self.stretch.origin = pos;
        }
        self.stretch.pressed?;
        // Either cancels the drag (the shared gesture law): let go, it sets nothing.
        let cancel = pinch || held && !down;
        if down && !cancel {
            if let (Some(p), Some(o)) = (pos, self.stretch.origin)
                && (self.stretch.len.is_some() || gesture::left_calm(o, Some(p), m.u(1.0)))
            {
                let tick = axis.tick(p.x - m.u(STRETCH_OFF));
                self.stretch.len = Some(stretch_len(tick, origin, snap, src));
            }
            return None;
        }
        let held = now - self.stretch.pressed.take().unwrap_or(now);
        self.stretch.released = Some((now, grow(held)));
        self.stretch.origin = None;
        self.stretch.len.take().filter(|_| !cancel)
    }

    /// The stretch handle at the end line `end_x`: a 1 U `text` disc with a 1 px `background` α 0.3 rim, grown while
    /// pressed, wobbling once let go (the graph lane knob's law).
    fn paint_stretch(&mut self, ui: &Ui, cx: &UiCtx, f: &Frame, end_x: f32, now: f64) {
        let (m, c) = (cx.m, &cx.theme.colors);
        let px = 1.0 / m.ppp();
        let (growth, moving) = press_growth(self.stretch.pressed, self.stretch.released, now);
        if moving {
            ui.ctx().request_repaint();
        }
        let centre = stretch_box(f, end_x, m).center();
        let r = (m.u(STRETCH_D) + growth * px) / 2.0;
        let painter = ui.painter_at(f.grid);
        painter.circle_filled(centre, r, c.text.to_color32());
        let rim = c.background.to_color32().gamma_multiply(0.3);
        painter.circle_stroke(centre, r + px / 2.0, (px, rim));
    }

    /// 2: the waveform's columns over the sample's extent.
    #[allow(clippy::too_many_arguments)]
    fn paint_wave(
        &mut self,
        painter: &Painter,
        f: &Frame,
        axis: TimeAxis,
        source: &Waveform,
        wave: Wave,
        fill: Color32,
        m: &Metrics,
    ) {
        let (x0, n) = pixel_columns(f.grid, m);
        let px = 1.0 / m.ppp();
        let bpp = f64::from(px / axis.beat_w);
        let left = axis.tick(x0) / f64::from(PPQ);
        let end = wave.end();
        let cols = self.peaks.get(source, wave, left, bpp, n);
        let rows = wave_rows(f.value, cols.len() == 2);
        let mut mesh = Mesh::default();
        for (col, &row) in cols.iter().zip(&rows) {
            for (i, &peak) in col.iter().enumerate() {
                let beat = left + i as f64 * bpp;
                if beat < -1e-9 || beat >= end - 1e-9 {
                    continue;
                }
                add_column(&mut mesh, x0 + i as f32 * px, peak, row, px, fill);
            }
        }
        painter.add(mesh);
    }

    /// 7: the current lane's curve and points, the selection's guides and the right-edge handle.
    #[allow(clippy::too_many_arguments)]
    fn paint_points(
        &self,
        painter: &Painter,
        cx: &UiCtx,
        f: &Frame,
        axis: TimeAxis,
        (pts, sel): &(Vec<Point>, Vec<bool>),
        max: u16,
        ink: Color32,
        now: f64,
    ) {
        let (m, text) = (cx.m, cx.theme.colors.text.to_color32());
        let px = 1.0 / m.ppp();
        let on = |i: usize| sel.get(i) == Some(&true);
        let seg = |i: usize| if on(i) || on(i + 1) { text } else { ink };
        curve(painter, f, axis, pts, max, LINE_PX * px, seg);
        let k = self.anim_scale(now);
        for (i, p) in pts.iter().enumerate() {
            let centre = pos2(axis.x(p.tick.into()), f.point_y(p.value, max));
            let r = m.u(POINT_R) * if on(i) { k } else { 1.0 };
            painter.circle_filled(centre, r, if on(i) { text } else { ink });
        }
        let Some(bx) = self.sel_box(f, axis, pts, sel, max) else {
            return;
        };
        let g = f.grid;
        let v = |x: f32| Rect::from_x_y_ranges(m.floor_px(x)..=m.floor_px(x) + px, g.y_range());
        let h = |y: f32| Rect::from_x_y_ranges(g.x_range(), m.floor_px(y)..=m.floor_px(y) + px);
        let mut guides = vec![v(bx.left()), h(bx.top())];
        if bx.width() > px {
            guides.push(v(bx.right()));
        }
        if bx.height() > px {
            guides.push(h(bx.bottom()));
        }
        for r in guides {
            painter.rect_filled(r, 0.0, text);
        }
        if let Some(handle) = self.scale_handle(Some(bx), sel, m).filter(|_| !self.quiet) {
            painter.circle_filled(handle.center(), handle.width() / 2.0, text);
        }
    }

    /// The selected points' radius factor: pop-in 1.0515·sin(1.885·(1 − a)); dragged 2 − 0.6a (a ≥ 0.01); the release
    /// wobble 1 + (1 − a)·sin(2π·(1 − a)).
    fn anim_scale(&self, now: f64) -> f32 {
        let Some((anim, t0)) = self.anim else {
            return 1.0;
        };
        let a = pop_in(ANIM_RATE, now - t0);
        match anim {
            Anim::Pop if a > 0.0 => POP_GAIN * (POP_W * (1.0 - a)).sin(),
            Anim::Grow => {
                let a = ANIM_RATE.powf(((now - t0) * 60.0) as f32).max(DRAG_FLOOR);
                2.0 - 0.6 * a
            }
            Anim::Wobble => 1.0 + (1.0 - a) * (std::f32::consts::TAU * (1.0 - a)).sin(),
            Anim::Pop => 1.0,
        }
    }

    /// `Copy` and `Delete` over the point selection in a [`FloatingRow`] (waiting for a marquee). `Copy` fills the
    /// clipboard; returns the points `Delete` leaves.
    #[allow(clippy::too_many_arguments)]
    fn floating_row(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        f: &Frame,
        axis: TimeAxis,
        pts: &[Point],
        max: u16,
        boxing: bool,
    ) -> Option<(Vec<Point>, Vec<bool>)> {
        let bx = self.sel_box(f, axis, pts, &self.sel, max);
        let handles: Vec<Rect> = self.scale_handle(bx, &self.sel, cx.m).into_iter().collect();
        let row_h = f.value.height() / 127.0;
        // The row floats over the touch frame (the box grown by 1.25 U, as the automation editor's), so a release
        // anywhere a touch binds the selection re-shows it.
        // A quiet selection has no row, which a release on it brings back neither.
        let sel = bx.filter(|_| !boxing && !self.quiet).map(|rect| Selection {
            rect: rect.expand(cx.m.u(HIT_R)),
            one_row: rect.height() < row_h,
            handles: &handles,
        });
        let view = GridView {
            grid: f.grid,
            map: [axis.origin, f.value.top(), axis.beat_w, f.value.height()],
        };
        let (i, _) = self
            .row
            .show(ui, "audio_point_button", sel, view, &OPS, cx)?;
        ui.ctx().request_repaint();
        if i == 0 {
            set_clipboard(ui.ctx(), points::copy(pts, &self.sel, max));
            return None;
        }
        let kept: Vec<Point> = (pts.iter().zip(&self.sel))
            .filter(|(_, s)| !**s)
            .map(|(p, _)| *p)
            .collect();
        let n = kept.len();
        Some((kept, vec![false; n]))
    }

    /// The `Paste` button while shown: a 1.5 U `menuFill` square with `Paste` in `text`; a tap pastes there.
    fn paste_button(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        pts: &[Point],
        max: u16,
        snap: Option<f64>,
    ) -> Option<(Vec<Point>, Vec<bool>)> {
        let copied = clipboard(ui.ctx());
        let (rect, tick) = self.paste.filter(|_| !copied.is_empty())?;
        let c = &cx.theme.colors;
        // It senses drags too, so a press on it never reaches the grid under it.
        let id = Id::new("audio_editor").with("paste");
        let response = ui.interact(rect, id, Sense::click_and_drag());
        ui.painter()
            .rect_filled(rect, 0.0, c.menu_fill.to_color32());
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Paste",
            cx.m.font_id(FontSize::Pt10),
            c.text.to_color32(),
        );
        if !response.clicked() {
            return None;
        }
        self.paste = None;
        Some(points::paste(pts, &copied, tick, snap, max))
    }

    /// The Snap button in `cell` ([`widgets::snap_button`]): a tap toggles snap, a long press or a right click opens
    /// the snap menu.
    fn snap_button(&mut self, ui: &mut Ui, id: Id, cell: Rect, cx: &UiCtx) {
        match widgets::snap_button(ui, id.with("snap"), cell, self.snap.on, cx) {
            Some(SnapTouch::Menu) => self.snap_menu = true,
            Some(SnapTouch::Toggle) => self.snap.on = !self.snap.on,
            None => {}
        }
    }

    /// The snap menu while open: one column, the current mode highlighted; a pick sets it and turns snap on.
    fn snap_menu(&mut self, ui: &Ui, id: Id, cell: Rect, cx: &UiCtx) {
        if !self.snap_menu {
            return;
        }
        let ctx = ui.ctx();
        let items: Vec<PopupItem<SnapMode>> = (SnapMode::ALL.iter().enumerate())
            .map(|(row, &mode)| PopupItem {
                current: mode == self.snap.mode,
                ..PopupItem::new(mode, 0, row, mode.label())
            })
            .collect();
        let result = popup::show(
            ctx,
            id.with("snap menu"),
            cell.center(),
            ctx.content_rect(),
            &items,
            cx,
        );
        if let Some(mode) = result.selected {
            self.snap = Snap { on: true, mode };
        }
        if result.close {
            self.snap_menu = false;
            ctx.request_repaint();
        }
    }
}

/// An envelope's curve: straight segments between its points (segment i in `color(i)`), none left of the first, a level
/// line from the last to the grid's right edge.
fn curve(
    painter: &Painter,
    f: &Frame,
    axis: TimeAxis,
    pts: &[Point],
    max: u16,
    width: f32,
    color: impl Fn(usize) -> Color32,
) {
    let at = |p: &Point| pos2(axis.x(p.tick.into()), f.point_y(p.value, max));
    for (i, w) in pts.windows(2).enumerate() {
        painter.line_segment([at(&w[0]), at(&w[1])], Stroke::new(width, color(i)));
    }
    if let Some(last) = pts.last() {
        let a = at(last);
        if a.x < f.grid.right() {
            let stroke = Stroke::new(width, color(pts.len() - 1));
            painter.line_segment([a, pos2(f.grid.right(), a.y)], stroke);
        }
    }
}

#[cfg(test)]
mod tests;
