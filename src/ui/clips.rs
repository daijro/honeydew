//! Playlist clips. Each track row shows its lane's clips as [`paint_clip`] draws them, rounded boxes: the current clip
//! has the light outline. The selection (one clip, or a marquee's group) is one pulsing `text` box with its capsule
//! handles (start, end, and a bar under an audio clip that does not loop, [`paint_handle`]) and the floating `Copy`,
//! `Delete`, `Snap`, `Edit`, `More...` above it. Dragging a clip moves it, dragging a handle resizes it (each clip of a
//! group). Positions snap to the ruler's Snap unit (its mode, Auto by the zoom); with Snap off, drags go to whole ticks
//! while the insert menu, `Copy` and `Snap` keep the mode's unit. Clips drag between tracks of any channel by lane
//! kind ([`Clips::drag_rows`]).
//!
//! Gestures, each decided from its touch-down by the shared law ([`gesture::is_long`], [`gesture::left_calm`]):
//! - Touch-down on a clip makes it the selection, its handles and row hidden until the touch ends. The current clip
//!   drags at once; another must be held 300 ms first, and a move before then cancels the touch and hides the
//!   selection (the playlist does not scroll then).
//! - A tap on a clip makes it the current clip, its track the current track and its channel the keyboard's; it
//!   deselects a clip that was current and selected already, else selects it. A tap on the current clip released
//!   < 0.3 s after the previous clip tap selects nothing: it opens the clip's editor, as `Edit` does (a double tap).
//!   The timer starts at egui time 0, so it has run out 0.3 s after the app starts.
//! - A long press on a clip selects it with its handles and row, the current clip unchanged.
//! - A move's release (as the piano roll's) is neither a tap nor a long press: the moved clip or group stays selected,
//!   the selection box pulsing, with its handles and row hidden (a quiet selection, whose handles take no touch and
//!   whose group frame only its clips' touches reach; a tap on empty track space still only deselects it). A tap on a
//!   clip then selects it with them; a long press, a box, `New Clip`, a paste or a key shows a selection again.
//! - A tap on empty track space only deselects a showing selection; else on the current track it shows the insert menu
//!   `New Clip` / `Paste` at the snapped beat ([`Clips::insert_menu`]), and a second tap on that cell hides it; on
//!   another track it makes that one current. The current track goes with the keyboard's channel (its main track until
//!   a tap picks another of its tracks).
//! - A long press on empty track space (any track) shows the menu there from 300 ms on, while the finger holds still,
//!   and keeps it at the lift; dragging on makes it a marquee instead, and hides the menu. A right click on empty track
//!   space shows the menu there too.

use eframe::egui::{
    self, Align2, Color32, FontId, Id, Mesh, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
    pos2, vec2,
};
use honeydew_audio::drums::SPEED;

use crate::app::note_record;
use crate::metrics::{FontSize, Metrics};
use crate::project_files::ProjectFiles;
use crate::sample_cache::{self, SampleCache, Waveform};
use crate::song::{
    Channel, ChannelKind, Clip, DrumGrid, Event, Lane, LaneKind, MAX_PLAYS_A_PIXEL, PPQ, Song,
    TimeSig,
};
use crate::theme::{Rgba, Theme, ThemeColors, channel_color};
use crate::ui::audio_editor::{
    self, Action, Wave, add_column, columns, stretch_of, stretch_ratio, wave_rows,
};
use crate::ui::automation::pop_scale;
use crate::ui::buttons::bubble_button;
use crate::ui::edit_keys::{self, EditKey};
use crate::ui::graph_lane::{grow, press_growth};
use crate::ui::insert::{self, Tap};
use crate::ui::main_screen::track_rows;
use crate::ui::piano_roll::notes::{Snap, auto_grid};
use crate::ui::popup::{self, PopupItem};
use crate::ui::preview::ClipKey;
use crate::ui::step_seq::pop_in;
use crate::ui::views::{AppCtx, Nav, Overlay};
use crate::ui::widgets::truncated;
use crate::ui::{UiCtx, gesture, primary_down_live};

/// The double-tap window: a tap on the current clip released this soon after the previous clip tap opens its editor.
const DOUBLE_TAP_S: f64 = 0.3;
/// The marquee's square while held, in U.
const MARQUEE_U: f32 = 3.0;
/// The marquee's fill: `text` at this alpha (its outline is `text`, opaque).
const MARQUEE_ALPHA: f32 = 0.2;
/// The floating row's pop-in: a ← 0.6·a per frame.
const ROW_POP: f32 = 0.6;
/// A hidden track's row, which Settings `Show hidden tracks` shows: `background` at this alpha over its header and clips,
/// under the playhead and the selection.
pub(crate) const HIDDEN_VEIL: f32 = 0.6;

/// The side of the square a touch held on empty grid grows, `elapsed` seconds after its touch-down: 3 U·k,
/// k = clamp(4.4333·elapsed − 0.33, 0, 1). It shows from about 74 ms and is 3 U at 300 ms.
pub(crate) fn hold_side(u: f32, elapsed: f64) -> f32 {
    MARQUEE_U * u * ((elapsed * 4.4333 - 0.33) as f32).clamp(0.0, 1.0)
}

/// The square of [`hold_side`] centred on `at`, `None` while it has no size yet.
pub(crate) fn hold_square(at: Pos2, u: f32, elapsed: f64) -> Option<Rect> {
    let side = hold_side(u, elapsed);
    (side > 0.0).then(|| Rect::from_center_size(at, Vec2::splat(side)))
}

/// A marquee from its long press on empty grid: the touch-down point, the finger, and whether the finger has dragged it
/// (beyond the calm zone after the long press, or at once with the selection modifier).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Marquee {
    pub from: Pos2,
    pub to: Pos2,
    pub dragged: bool,
}

impl Marquee {
    pub(crate) fn at(p: Pos2, dragged: bool) -> Self {
        Self {
            from: p,
            to: p,
            dragged,
        }
    }

    /// Follows the finger at `pos` (none on a lift's frame: where it was); dragged once it has left the calm zone
    /// about the touch-down ([`gesture::left_calm`], `u` points a U).
    pub(crate) fn follow(&mut self, pos: Option<Pos2>, u: f32) {
        if let Some(p) = pos {
            self.to = p;
            self.dragged |= gesture::left_calm(self.from, pos, u);
        }
    }

    /// The box drawn: a 3 U square on the finger until dragged, then the rectangle from the touch-down to the
    /// finger (screen points, no snapping).
    pub(crate) fn drawn(&self, u: f32) -> Rect {
        if self.dragged {
            Rect::from_two_pos(self.from, self.to)
        } else {
            Rect::from_center_size(self.to, Vec2::splat(MARQUEE_U * u))
        }
    }

    /// What it selects: the finger's point until dragged (so nothing on empty grid), then the rectangle, clamped
    /// to the grid. Touching counts.
    pub(crate) fn region(&self, grid: Rect) -> Rect {
        let rect = if self.dragged {
            Rect::from_two_pos(self.from, self.to)
        } else {
            Rect::from_min_max(self.to, self.to)
        };
        Rect::from_min_max(grid.clamp(rect.min), grid.clamp(rect.max))
    }
}

/// Paints a marquee box: `text` at α 0.2, outlined in `text` one hairline (ceil(U/16) px) wide inside it.
pub(crate) fn paint_marquee(painter: &Painter, rect: Rect, cx: &UiCtx) {
    let text = cx.theme.colors.text.to_color32();
    painter.rect(
        rect,
        0.0,
        text.gamma_multiply(MARQUEE_ALPHA),
        Stroke::new(cx.m.t(), text),
        StrokeKind::Inside,
    );
}

// The selection overlay; sizes in U.
/// A selection handle is a vertical capsule this wide and tall, plus 2 px each ([`capsule`]); its touch zone is
/// [`ZONE`] wide (the start handle's at most a third of the selection wide, so a narrow clip still moves from most of
/// its width).
const HANDLE: [f32; 2] = [0.42, 1.41];
const ZONE: f32 = 1.0;
/// The start handle's right edge is this far inside the selection box's left edge; the end handle's left edge this far
/// right of its right edge.
const START_IN: f32 = 0.5;
const END_OFF: f32 = 0.15;
/// The third (audio scale) handle: a bar this wide and tall (plus 2 px), centred on the selection box's right edge, its
/// top this far below the box.
const SCALE_BAR: [f32; 2] = [1.0, 0.56];
const SCALE_OFF: f32 = 0.12;
/// A handle's frame and its grip bar: `menuFill` at these alphas. The grip is this share of the handle's width and
/// height, centred.
const FRAME_A: f32 = 0.3;
const GRIP_A: f32 = 0.7;
const GRIP: [f32; 2] = [0.18, 0.44];
/// The third handle's `↔`: `text` mixed this much toward `menuFill`.
const ARROW_MIX: f32 = 0.7;
/// Under the mouse a handle is this much larger.
const HOVER: f32 = 1.05;
/// Clip boxes have rounded corners of this radius.
pub(crate) const CORNER: f32 = 0.15;
/// The selection fill: `text` at α 0.15 + 0.05·cos(2π·p), p advancing this much a drawn frame (an 80-frame cycle).
const PULSE: [f32; 3] = [0.15, 0.05, 0.0125];
/// The handles' pop-in: a ← 0.6·a per frame.
const HANDLE_RATE: f32 = 0.6;
/// Floating-menu bubbles have a 0.2 U gap, with room for their pop-in animation.
pub(crate) const BUTTON_D: f32 = 1.4;
const ROW_B: f32 = 1.5;
const ROW_SLOT: f32 = 1.6;
const ROW_M: f32 = 0.2;
/// Labels shrink until each fits this share of a button's diameter.
const LABEL_FIT: f32 = 0.85;
// The clip drawing.
/// The name's strip, the top of the body, in U: the miniature starts this far below the clip box's top.
const NAME_H: f32 = 0.7;
/// The name starts this many device pixels right of the clip box (or at the grid's left edge), and ends this many
/// left of its right end.
const NAME_IN_PX: f32 = 5.0;
const NAME_END_PX: f32 = 2.0;
/// The name's middle ellipsis keeps up to this many tail characters.
const NAME_K: usize = 5;
/// The miniature ends this many t above the clip box's bottom.
const CONTENT_END_T: f32 = 6.0;
/// The frame: `background` at this alpha, a device pixel around the clip box.
const FRAME_ALPHA: f32 = 0.3;
/// Where the middle stop of the miniature's tint sits.
const TINT_MID: f32 = 0.66;
/// An automation miniature and an audio waveform sample the tint here.
const CURVE_TINT: f32 = 0.8;

/// Song ticks to x and back. `origin` is the x of tick 0 (left of the grid once scrolled).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeAxis {
    pub origin: f32,
    /// Points per beat.
    pub beat_w: f32,
}

impl TimeAxis {
    pub fn x(self, tick: f64) -> f32 {
        self.origin + (tick / f64::from(PPQ)) as f32 * self.beat_w
    }

    pub fn tick(self, x: f32) -> f64 {
        f64::from((x - self.origin) / self.beat_w) * f64::from(PPQ)
    }
}

/// The Auto snap grid in ticks: the piano roll's Auto grid for the timeline's beat width. A step (1/d beat) may be a
/// fractional tick; snapped positions are rounded to whole ticks.
pub fn grid_ticks(axis: TimeAxis, sig: TimeSig, m: &Metrics) -> f64 {
    auto_grid(axis.beat_w, m.u(1.0), sig)
}

/// The scroll (the beat at the grid's left edge) and zoom (a power of two of `beat_w0`, the default beat width
/// in points, kept in `range`) after zooming by `factor` around the point `anchor` points right of the grid's left
/// edge.
pub fn zoomed(
    scroll: f64,
    zoom: f32,
    factor: f32,
    anchor: f32,
    beat_w0: f32,
    range: [f32; 2],
) -> (f64, f32) {
    let new = (zoom + factor.log2()).clamp(range[0], range[1]);
    let beats = |z: f32| f64::from(anchor / (beat_w0 * z.exp2()));
    ((scroll + beats(zoom) - beats(new)).max(0.0), new)
}

/// A clip of the playlist: the channel, the lane in it, and the clip's id in the lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRef {
    pub channel: usize,
    pub lane: usize,
    pub id: u32,
}

impl ClipRef {
    /// Where its editor opens: `Edit`'s navigation, and a double tap's.
    fn edit(self) -> Nav {
        Nav::EditClip(self.into())
    }
}

/// The same clip, as the editors name it.
impl From<ClipRef> for ClipKey {
    fn from(ClipRef { channel, lane, id }: ClipRef) -> Self {
        Self { channel, lane, id }
    }
}

impl From<ClipKey> for ClipRef {
    fn from(ClipKey { channel, lane, id }: ClipKey) -> Self {
        Self { channel, lane, id }
    }
}

/// A take being recorded: its channel, its track and its span in song ticks, which grows as the take does. `clip`: the
/// clip it records into, which shows only as this bar meanwhile (a notes or drums take's with its notes live, a
/// Honeydew addition: [`paint_live`]); an audio take has none until it stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveTake {
    pub channel: usize,
    pub lane: usize,
    pub start: f64,
    pub end: f64,
    pub clip: Option<u32>,
}

/// The playlist's clip selection and the gesture on it.
#[derive(Default)]
pub struct Clips {
    /// The paint slot [`Self::show`] leaves between the clips and their selection chrome, for the playhead line
    /// (clips, then the line, then the grid's children).
    pub(crate) line_slot: Option<egui::layers::ShapeIdx>,
    /// The clip selected with its handles and floating row.
    pub selected: Option<ClipRef>,
    /// The selection (the clip or the group) is quiet: kept, its handles and floating row hidden, as a move's release
    /// leaves it (module docs).
    quiet: bool,
    /// The loop repeat a touch-down on a clip's body was on ([`repeat_at`]): while that clip is selected with its
    /// handles, its other repeats are dimmed. Another selection's is repeat 0.
    pick: Option<(ClipRef, u64)>,
    /// The track a tap last made current ([`Self::current_track`]). Each track keeps its own current clip, in the song
    /// (DESc@24, [`Self::current_clip`]).
    pub(crate) track: Option<(usize, usize)>,
    /// When the last tap on a clip was released (egui time): the double-tap timer. It starts at 0, the egui time the
    /// app starts at, so it runs out 0.3 s after the playlist is created.
    last_tap: f64,
    /// A touch until it is decided.
    touch: Option<Touch>,
    /// The insert menu (`New Clip`, `Paste`) on track (channel, lane), for this snapped tick ([`Self::insert_menu`]).
    insert: Option<(usize, usize, f64)>,
    /// Several clips a marquee selected (none of them `selected`): one selection box with its handles, one floating row
    /// over them all.
    pub(crate) group: Vec<ClipRef>,
    /// The marquee from a long press on empty space: its selection is live.
    marquee: Option<Marquee>,
    /// A drag on the selected clip or the group.
    drag: Option<Drag>,
    /// The `More...` dropdown is open, centred here.
    more: Option<Pos2>,
    /// The floating buttons over the selected clip.
    pub(crate) row: FloatingRow,
    /// The selection fill's phase p, a drawn frame's step on per frame the selection shows.
    pulse: f32,
    /// Since when the selection's handles show (their pop-in), the handle held since when, and the one let go when with
    /// the growth it had (its wobble): input times.
    shown: Option<f64>,
    held: Option<(Part, f64)>,
    released: Option<(Part, f64, f32)>,
    /// The grid cursor (song tick, track): where the last touched clip was touched, where the keymap's `paste` and
    /// `open` act ([`Self::keys`]); `None` before any: tick 0 on the current track.
    cursor: Option<(f64, (usize, usize))>,
    /// The clip clipboard (`copy`, `cut`): the clips last copied, each with the track it was on (which follows the
    /// tracks as they move, [`Self::rekey`]) and that track's kind. The song's: a new song starts without it.
    clipboard: Vec<(ClipRef, LaneKind, Clip)>,
    /// Whether the playlist's scroller has the touch and it left its calm zone (on either axis, 0.5 U each); `None`
    /// when the scroller has no such touch. A touch on empty lane space decides by it. The host sets it each frame.
    pub(crate) left_calm: Option<bool>,
    /// What the last `Delete` left empty, for the app to ask about after the frame ([`emptied`]).
    pub(crate) emptied: Option<Emptied>,
}

/// The channels and tracks a playlist `Delete` left with no clip, by index: the app asks whether they go too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emptied {
    pub channels: Vec<usize>,
    /// (channel, lane), of channels that stay.
    pub lanes: Vec<(usize, usize)>,
}

/// What deleting the clips `targets` (gone from `song` now) left empty. A channel with no clip left on any track, never
/// the master or a send; else each track of a target clip left with none, all but the first when every track of the
/// channel is (a channel keeps one). `None` when nothing.
pub(crate) fn emptied(song: &Song, targets: &[ClipRef]) -> Option<Emptied> {
    let mut e = Emptied::default();
    for (c, ch) in song.channels.iter().enumerate() {
        let mut lanes: Vec<usize> = (targets.iter())
            .filter(|r| r.channel == c && ch.lanes.get(r.lane).is_some_and(|l| l.clips.is_empty()))
            .map(|r| r.lane)
            .collect();
        lanes.sort_unstable();
        lanes.dedup();
        if lanes.is_empty() {
            continue;
        }
        let kept = c == 0 || ch.kind == ChannelKind::EffectTrack;
        if !kept && ch.lanes.iter().all(|l| l.clips.is_empty()) {
            e.channels.push(c);
            continue;
        }
        if lanes.len() == ch.lanes.len() {
            lanes.remove(0);
        }
        e.lanes.extend(lanes.into_iter().map(|l| (c, l)));
    }
    (!e.channels.is_empty() || !e.lanes.is_empty()).then_some(e)
}

/// What a touch-down landed on: a clip (by its body or a handle of the selected one), a track's empty space, or the
/// showing group: its frame (`Part::Body`, on one of its clips or not) or a handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Clip(ClipRef, Part),
    Lane(usize, usize),
    Group(Option<ClipRef>, Part),
}

/// A touch from its touch-down until a drag takes it over or it ends.
#[derive(Clone, Copy)]
struct Touch {
    on: Target,
    origin: Pos2,
    since: f64,
    /// The selection at the touch-down (the touched clip has been the selection since).
    prior: Option<ClipRef>,
    /// The touched clip was current: it drags at once.
    current: bool,
    /// A selection (a clip or the group) was showing.
    shown: bool,
    /// It left the calm zone too early to drag: nothing more until the lift.
    cancelled: bool,
    /// The insert menu that showed at the touch-down (its press hid it).
    menu: Option<(usize, usize, f64)>,
}

#[derive(Clone)]
struct Drag {
    part: Part,
    /// Each dragged clip, as the drag found it and as it last left it. Anything else means the song changed under
    /// the drag (an undo), which ends it.
    clips: Vec<(ClipRef, Span, Span)>,
    /// The tick under the press.
    from: f64,
    /// The drag reported an ongoing edit, so its release reports the final one, even back where it started.
    edited: bool,
    /// The press's y: the track row under it is where a body drag's rows count from.
    y: f32,
    /// How many rows down the track list the drag has put the clips from where it found them.
    rows_down: isize,
    /// The track the current clip was dragged onto, with that track's own current clip before (it gets it back when
    /// the clip moves on).
    visit: Option<(usize, usize, u32)>,
    /// The clips and each track the rows moved them off or onto, as the first move found them: they come back whole
    /// once every clip is back on its own track, so a drag back where it started changed nothing (no clip id, no
    /// undo step).
    before: Option<(Vec<ClipRef>, Vec<TrackLane>)>,
}

/// A track (channel, lane) and its lane.
type TrackLane = (usize, usize, Lane);

impl Drag {
    /// Each dragged clip is as the drag last left it: else the song changed under the drag (an undo), which ends it.
    fn unchanged(&self, song: &Song) -> bool {
        (self.clips.iter()).all(|&(r, _, last)| find(song, r).is_some_and(|c| Span::of(c) == last))
    }

    /// The drag's end: the third handle scales the clip's sample with it.
    fn scale_sample(&self, song: &mut Song, samples: &mut SampleCache, files: &ProjectFiles) {
        if let (Part::Scale, [(r, orig, last)]) = (self.part, &self.clips[..]) {
            let tempo = song.tempo_bpm;
            let sample = find(song, *r)
                .and_then(|c| audio_wave(samples, files, c, tempo))
                .map(|(source, _)| (source.data.frames(), source.data.sample_rate));
            if let Some(sample) = sample {
                stretch_clip(song, *r, sample, last.visible / orig.visible);
            }
        }
    }
}

/// The floating buttons, left to right.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipOp {
    Copy,
    Delete,
    Snap,
    Edit,
    More,
}

const OPS: [(ClipOp, &str); 5] = [
    (ClipOp::Copy, "Copy"),
    (ClipOp::Delete, "Delete"),
    (ClipOp::Snap, "Snap"),
    (ClipOp::Edit, "Edit"),
    (ClipOp::More, "More..."),
];

impl Clips {
    /// Draws and edits the clips of the visible track rows. `rows` gives each row's channel, lane and lane rect; `grid`
    /// is the area right of the track headers, which clips are clipped to. While Preview plays `preview` alone, every
    /// other clip is dimmed as a muted one. The `takes` being recorded show over the clips as live bars
    /// ([`paint_take`]), a notes or drums take's clip with its notes as they record ([`paint_live`], Honeydew's own).
    /// Song edits are reported through `app.song_edited`; a clip's `Edit`, or a double tap on it, returns its editor's
    /// navigation, as does `New Clip` on an audio track (the sample picker). The insert menu,
    /// `Copy` and `Snap` round to `snap`'s unit (round(beat/unit)·unit), and drags too while it is on (whole ticks
    /// while off).
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        grid: Rect,
        rows: &[(usize, usize, Rect)],
        axis: TimeAxis,
        preview: Option<ClipKey>,
        takes: &[LiveTake],
        snap: Snap,
    ) -> Option<Nav> {
        let (m, sig) = (app.cx.m, app.song.time_sig);
        let auto = grid_ticks(axis, sig, m);
        let (snap, drag_unit) = (snap.grid(auto, sig), snap.active(auto, sig).unwrap_or(1.0));
        let bar = app.song.time_sig.bar_beats();
        // A clip that went away (undo, another song), or whose track no longer shows (`Hide track`), takes its selection
        // and gesture with it.
        let show = app.settings.show_hidden_tracks;
        let kept = |r: ClipRef| {
            find(app.song, r).is_some()
                && (show || !app.song.channels[r.channel].lanes[r.lane].hidden)
        };
        if self.selected.is_some_and(|r| !kept(r)) {
            self.deselect();
        }
        self.group.retain(|&r| kept(r));
        // A current track or grid cursor whose track no longer shows moves to the nearest track that does (the one below
        // on a tie): `paste` and `Add clip` go there.
        if !show {
            let all = track_rows(app.song, true);
            let hidden = |t: (usize, usize)| {
                (all.iter().position(|r| (r.channel, r.lane) == t)).filter(|&p| all[p].hidden)
            };
            if let Some(p) = hidden(self.track_for(*app.selected)) {
                let near = (all.iter().enumerate().filter(|(_, r)| !r.hidden))
                    .min_by_key(|&(i, _)| (i.abs_diff(p), i < p));
                if let Some((_, r)) = near {
                    self.set_track(app.selected, r.channel, r.lane);
                }
            }
            if self.cursor.is_some_and(|(_, t)| hidden(t).is_some()) {
                self.cursor = None;
            }
        }
        if self.group.len() == 1 {
            self.selected = self.group.pop();
        }

        let (now, pressed, pos, held) = ui.input(|i| {
            let p = &i.pointer;
            // Unlike `latest_pos`, `interact_pos` is kept through a `PointerGone` in the release frame (a touch's lift).
            (
                i.time,
                p.primary_pressed(),
                p.interact_pos(),
                p.primary_down(),
            )
        });
        // Every frame, for its latch: a press held over a focus loss is up, and decides nothing.
        let down = primary_down_live(ui, Id::new("clip_touch"));
        let stale = held && !down || ui.input(|i| !i.focused || crate::ui::focus_lost(i));
        let multi = ui.input(|i| i.multi_touch().is_some());
        // The menu a touch-down finds: a tap on its own cell only hides it.
        let menu = self.insert;
        if multi || stale {
            self.insert = None;
        }
        // A move's release leaves its selection quiet (module docs); a pinch puts the clips back as they were.
        if !down && !multi && self.drag.as_ref().is_some_and(|d| d.part == Part::Body) {
            self.quiet = true;
        }
        self.quiet &= self.selected.is_some() || !self.group.is_empty();
        // What a touch-down lands on: an empty lane, a clip or a selected clip's handle (the later, the higher).
        let mut landed = None;
        for &(channel, lane, row) in rows {
            let id = Id::new(("clip_lane", channel, lane));
            let tap = ui.interact(row.intersect(grid), id, Sense::click());
            if pressed && tap.is_pointer_button_down_on() {
                landed = Some(Target::Lane(channel, lane));
            }
            // A right click there shows the insert menu.
            if tap.secondary_clicked()
                && let Some(p) = tap.interact_pointer_pos()
            {
                self.insert = Some((channel, lane, cell(axis.tick(p.x), snap)));
            }
        }

        // The clips, the selected one last: on top, and first to take the pointer.
        let mut blocks: Vec<(ClipRef, Rect)> = rows
            .iter()
            .flat_map(|&(channel, lane, row)| {
                let clips = app
                    .song
                    .channels
                    .get(channel)
                    .and_then(|c| c.lanes.get(lane));
                clips.into_iter().flat_map(|l| &l.clips).map(move |clip| {
                    let r = ClipRef {
                        channel,
                        lane,
                        id: clip.id,
                    };
                    (r, clip_rect(clip, row, axis))
                })
            })
            .filter(|(_, rect)| rect.intersects(grid))
            .collect();
        blocks.sort_by_key(|&(r, _)| Some(r) == self.selected);
        let painter = ui.painter_at(grid);
        let tempo = app.song.tempo_bpm;
        // A touch on a clip or in the group's frame hides the selection's handles (and its floating row) until it ends;
        // a handle's own does not. A quiet selection has none.
        let body = self.quiet
            || (self.drag.as_ref()).is_some_and(|d| d.part == Part::Body)
            || (self.touch).is_some_and(|t| {
                matches!(
                    t.on,
                    Target::Clip(_, Part::Body) | Target::Group(_, Part::Body)
                )
            });
        let current = self.current_clip(app);
        // The current clip's outline goes over every row and clip, after them all ([`paint_outline`]).
        let mut outline = None;
        for &(r, rect) in &blocks {
            // A clip being recorded shows only as its take's live bar (with its notes live: [`paint_live`]).
            let live =
                |t: &LiveTake| (t.channel, t.lane, t.clip) == (r.channel, r.lane, Some(r.id));
            if let Some(clip) = find(app.song, r).filter(|_| !takes.iter().any(live)) {
                let silent =
                    preview.is_some_and(|k| (k.channel, k.lane, k.id) != (r.channel, r.lane, r.id));
                let wave = audio_wave(&mut app.audio.samples, app.files, clip, tempo);
                let channel = &app.song.channels[r.channel];
                let pick = self.pick.filter(|p| p.0 == r).map_or(0, |p| p.1);
                let look = Look {
                    current: current == Some(r),
                    dimmed: clip.muted || silent,
                    picked: (self.selected == Some(r) && !body).then_some(pick),
                };
                if look.current {
                    outline = Some((clip_box(rect, m), look.dimmed));
                }
                paint_clip(
                    &painter,
                    &app.cx,
                    channel,
                    &channel.lanes[r.lane],
                    clip,
                    wave.as_ref().map(|(source, wave)| (source, *wave)),
                    look,
                    clip_box(rect, m),
                    axis,
                );
            }
            let response = ui.interact(
                rect.intersect(grid),
                Id::new(("clip", r.channel, r.lane, r.id)),
                Sense::click_and_drag(),
            );
            if pressed && response.is_pointer_button_down_on() {
                landed = Some(Target::Clip(r, Part::Body));
            }
        }

        for take in takes {
            let row = rows
                .iter()
                .find(|&&(c, l, _)| (c, l) == (take.channel, take.lane));
            if let (Some(&(.., row)), Some(channel)) = (row, app.song.channels.get(take.channel)) {
                let x = axis.x(take.start)..=axis.x(take.end);
                let rect = Rect::from_x_y_ranges(x, row.y_range());
                let lane = &channel.lanes[take.lane];
                let clip = (take.clip)
                    .filter(|_| matches!(lane.kind, LaneKind::Notes | LaneKind::Drums))
                    .and_then(|id| lane.clips.iter().find(|c| c.id == id));
                match clip {
                    Some(clip) => {
                        let clip = note_record::live(clip, take.end as u64, bar);
                        paint_live(&painter, &app.cx, channel, lane, &clip, rect, axis);
                    }
                    None => paint_take(&painter, &app.cx, channel, rect),
                }
            }
        }

        // A hidden track's clips (Settings `Show hidden tracks`), dimmed under the playhead and the selection.
        let veil = (app.cx.theme.colors.background.to_color32()).gamma_multiply(HIDDEN_VEIL);
        for &(channel, lane, row) in rows {
            let track = app
                .song
                .channels
                .get(channel)
                .and_then(|c| c.lanes.get(lane));
            if track.is_some_and(|l| l.hidden) {
                painter.rect_filled(row, 0.0, veil);
            }
        }
        if let Some((b, dimmed)) = outline {
            paint_outline(&painter, &app.cx, b, dimmed);
        }
        self.line_slot = Some(painter.add(egui::Shape::Noop));
        // The selection overlay: one pulsing `text` box over every selected clip, gaps included, no outline. While a
        // marquee is live it is the only box drawn: the selection shows on its release; its frame still takes touches
        // and places the floating row.
        let selection = selection_box(app.song, &self.targets(), rows, axis, 1.0 / m.ppp(), show);
        let sbox = selection.filter(|_| self.marquee.is_none());
        paint_pulse(&painter, &app.cx, sbox, m.u(CORNER), &mut self.pulse);
        // A touch-down in the group's frame acts on the group whole; in a quiet group's, only one on its clips.
        let frame = selection.filter(|_| !self.group.is_empty());
        let grouped = |on| matches!(on, Target::Clip(r, _) if self.group.contains(&r));
        if let (Some(on), Some(frame), Some(p)) = (landed, frame, pos)
            && frame.contains(p)
            && (!self.quiet || grouped(on))
        {
            landed = Some(Target::Group(
                match on {
                    Target::Clip(r, _) => Some(r),
                    _ => None,
                },
                Part::Body,
            ));
        }
        let scale = (self.selected.filter(|_| self.group.is_empty()))
            .and_then(|r| {
                Some((
                    app.song.channels.get(r.channel)?.lanes.get(r.lane)?,
                    find(app.song, r)?,
                ))
            })
            .is_some_and(|(lane, clip)| scalable(lane, clip));
        // The handles pop in from the frame they show.
        self.shown = (sbox.is_some() && !body).then(|| self.shown.unwrap_or(now));
        let mut handles = Vec::new();
        let shown_box = sbox.filter(|_| !self.quiet);
        for (part, rect) in shown_box.map_or(Vec::new(), |b| handle_rects(b, scale, m)) {
            // The right handle's drag hides the third, the third's the other two.
            let hidden = match self.held.map(|h| h.0) {
                Some(Part::End) => part == Part::Scale,
                Some(Part::Scale) => part != Part::Scale,
                _ => false,
            };
            if !body && !hidden {
                let anim = self.handle_anim(part, now);
                paint_handle(&painter, &app.cx, rect, anim, part == Part::Scale);
                handles.push(rect);
            }
            let zone = handle_zone(part, rect, shown_box.unwrap_or(rect), m);
            let response = ui.interact(
                zone.intersect(grid),
                Id::new(("clip_handle", part)),
                Sense::drag(),
            );
            if pressed && response.is_pointer_button_down_on() {
                landed = Some(match self.selected {
                    Some(r) => Target::Clip(r, part),
                    None => Target::Group(None, part),
                });
            }
        }
        if self.marquee.is_some_and(|marquee| {
            marquee.dragged || crate::ui::gesture::left_calm(marquee.from, pos, m.u(1.0))
        }) {
            self.insert = None;
        }
        let (on_menu, picker) = self.insert_menu(ui, app, grid, rows, axis, (snap, bar), pressed);
        if on_menu {
            landed = None;
        }

        // The touch-down, then the touch until it is decided (module docs). With the selection modifier, one on
        // empty space starts a marquee at once.
        let shift = ui.input(|i| i.modifiers.shift);
        // The keymap's `editorSelect` (Ctrl/Cmd held) does so wherever the touch lands.
        let select = crate::keymap::held(ui.ctx(), "editorSelect");
        if let (Some(on), Some(p)) = (landed, pos)
            && (select || shift && matches!(on, Target::Lane(..)))
        {
            landed = None;
            self.marquee = Some(Marquee::at(p, true));
        }
        if let Some(on) = landed.filter(|_| down && !multi && self.drag.is_none())
            && let Some(origin) = pos
        {
            let (prior, shown) = (
                self.selected,
                (self.selected.is_some() || !self.group.is_empty()) && !self.quiet,
            );
            if let Target::Clip(_, part) | Target::Group(_, part) = on
                && part != Part::Body
            {
                self.held = Some((part, now));
            }
            let current = match on {
                Target::Clip(r, part) => {
                    self.cursor = Some((axis.tick(origin.x).max(0.0), (r.channel, r.lane)));
                    (self.selected, self.group) = (Some(r), Vec::new());
                    if part == Part::Body {
                        let tick = axis.tick(origin.x);
                        self.pick = find(app.song, r).map(|c| (r, repeat_at(c, tick)));
                    }
                    self.current_clip(app) == Some(r)
                }
                _ => false,
            };
            self.touch = Some(Touch {
                on,
                origin,
                since: now,
                prior,
                current,
                shown,
                cancelled: false,
                menu,
            });
        }
        let mut nav = None;
        if let Some(touch) = self.touch.take() {
            // A press held over a focus loss is no lift: it decides nothing, and ends below.
            if stale {
                self.touch = Some(touch);
            } else {
                nav = self.decide(app, touch, (pos, now, down), rows, (axis, snap));
            }
            if let Some(t) = self.touch.filter(|t| matches!(t.on, Target::Lane(..))) {
                // A held touch on empty space grows the marquee's square.
                ui.ctx().request_repaint();
                if let Some(square) = hold_square(t.origin, m.u(1.0), now - t.since) {
                    paint_marquee(&painter, square, &app.cx);
                }
            }
            if multi || stale {
                // A pinch only zooms, and a press held over a focus loss decides nothing: back as it was.
                if let Some(t) = self.touch.take()
                    && matches!(t.on, Target::Clip(..))
                {
                    self.selected = t.prior;
                }
            }
        }
        // The marquee: the clips it touches on the visible tracks are the selection, every frame.
        if let Some(mut marquee) = self.marquee.take() {
            marquee.follow(pos, m.u(1.0));
            let region = marquee.region(grid);
            let touched = (blocks.iter()).filter(|(_, rect)| rect.intersects(region));
            self.set_selection(touched.map(|&(r, _)| r).collect());
            if marquee.dragged {
                paint_marquee(&painter, marquee.drawn(m.u(1.0)), &app.cx);
            }
            // Held still it is the insert menu's long press: the menu shows at its touch-down, and stays at its
            // lift; dragged, it is a marquee only.
            self.insert = None;
            if !(multi || stale || marquee.dragged)
                && let Some(&(channel, lane, _)) =
                    rows.iter().find(|r| r.2.y_range().contains(marquee.from.y))
            {
                self.insert = Some((channel, lane, cell(axis.tick(marquee.from.x), snap)));
                ui.ctx().request_repaint();
            }
            if down && !multi && !stale {
                self.marquee = Some(marquee);
            }
        }
        // The floating row over the selected clip or the group, also one scrolled out sideways (the row waits at the
        // edge). Its view is the timeline's and the track list's: a scroll or zoom of either hides it. (Not the clip's
        // own row, which a tap on a clip of another track changes without any scroll.)
        let row_of = |r: ClipRef| {
            rows.iter()
                .find(|&&(c, l, _)| (c, l) == (r.channel, r.lane))
        };
        let target = match (self.selected, self.group.first()) {
            (Some(r), _) => row_of(r)
                .and_then(|&(.., row)| Some((r, clip_rect(find(app.song, r)?, row, axis), row))),
            (None, Some(&first)) => (frame.zip(self.group.iter().find_map(|&g| row_of(g))))
                .map(|(rect, &(.., row))| (first, rect, row)),
            (None, None) => None,
        };
        let same = |a: &ClipRef, b: &ClipRef| (a.channel, a.lane) == (b.channel, b.lane);
        let one_row = self.group.windows(2).all(|w| same(&w[0], &w[1]));
        let row = target.map_or(Rect::ZERO, |(.., row)| row);
        let view = GridView {
            grid,
            map: [
                axis.origin,
                list_top(app.song, rows, 1.0 / m.ppp(), show),
                axis.beat_w,
                row.height(),
            ],
        };
        // A quiet selection has no row, which a release on it brings back neither.
        let sel = target
            .filter(|_| !self.quiet && self.insert.is_none())
            .map(|(_, rect, _)| Selection {
                rect,
                one_row,
                handles: &handles,
            });
        let labels = OPS.map(|(_, label)| label);
        self.row.covered = app.covered;
        let clicked = self
            .row
            .show(ui, "clip_button", sel, view, &labels, &app.cx);
        let button = clicked.and_then(|(i, center)| self.act(app, i, center, snap));
        self.follow_drag(ui, app, axis, drag_unit, rows);
        // A handle let go wobbles from the growth it had.
        if self.touch.is_none() && self.drag.is_none() {
            let let_go = self.held.take().map(|(part, t)| (part, now, grow(now - t)));
            self.released = let_go.or(self.released);
        }
        self.more_menu(ui, app);
        button.or(nav).or(picker)
    }

    /// Handle `part`'s animation now (as the other editor handles', e.g. the audio editor's stretch handle):
    /// its pop-in scale 1.0515·sin(1.885·(1 − a)) from the frame the handles showed, a ← 0.6·a per frame; and its press
    /// growth in device pixels, up to 10 while held and a damped wobble once let go ([`press_growth`]).
    fn handle_anim(&self, part: Part, now: f64) -> (f32, f32) {
        let a = self.shown.map_or(0.0, |t| pop_in(HANDLE_RATE, now - t));
        let pop = if a > 0.0 { pop_scale(a) } else { 1.0 };
        let pressed = self.held.filter(|h| h.0 == part).map(|h| h.1);
        let released = self.released.filter(|r| r.0 == part).map(|r| (r.1, r.2));
        (pop, press_growth(pressed, released, now).0)
    }

    /// The current track: the one a tap last made current while its channel has the keyboard, else the keyboard
    /// channel's main track (the playlist's current track and the keyboard's channel go together).
    pub(crate) fn current_track(&self, app: &AppCtx) -> (usize, usize) {
        self.track_for(*app.selected)
    }

    /// [`Self::current_track`] with channel `keys` on the keyboard.
    fn track_for(&self, keys: usize) -> (usize, usize) {
        (self.track.filter(|t| t.0 == keys || t.0 == 0)).unwrap_or((keys, 0))
    }

    /// The current clip: the current track's own (each track keeps one, its lane's DESc@24), while it exists and its
    /// track shows. It is drawn as current, and the bottom editor shows it (the app; it collapses whenever the current
    /// track has none, so a track hidden under it closes it).
    pub(crate) fn current_clip(&self, app: &AppCtx) -> Option<ClipRef> {
        self.clip_for(app.song, *app.selected, app.settings.show_hidden_tracks)
    }

    /// [`Self::current_clip`] in `song` with channel `keys` on the keyboard; a hidden track has one only with
    /// `show_hidden` (Settings `Show hidden tracks`).
    pub(crate) fn clip_for(&self, song: &Song, keys: usize, show_hidden: bool) -> Option<ClipRef> {
        let (channel, lane) = self.track_for(keys);
        let track = song.channels.get(channel)?.lanes.get(lane)?;
        let id = track.file.current_clip_id;
        if track.hidden && !show_hidden {
            return None;
        }
        let r = ClipRef { channel, lane, id };
        find(song, r).map(|_| r)
    }

    /// Clip `r` becomes its track's current clip (DESc@24: view state, no edit) and its track the current one: a tap, a
    /// double tap, `Edit`, `Select clip`, `Add clip` and `New Clip`. Nothing else changes a track's current clip.
    pub(crate) fn make_current(&mut self, song: &mut Song, keys: &mut usize, r: ClipRef) {
        if let Some(lane) = lane_mut(song, r.channel, r.lane) {
            lane.file.current_clip_id = r.id;
        }
        self.set_track(keys, r.channel, r.lane);
    }

    /// Track (`channel`, `lane`) becomes current; its channel takes the keyboard and the rack (the master's keys play
    /// nothing).
    pub(crate) fn make_current_track(&mut self, app: &mut AppCtx, channel: usize, lane: usize) {
        self.set_track(app.selected, channel, lane);
    }

    fn set_track(&mut self, keys: &mut usize, channel: usize, lane: usize) {
        self.track = Some((channel, lane));
        *keys = channel;
    }

    /// Follows the tracks when channels or tracks move: `track` maps a track's old (channel, lane) to its new one, `None`
    /// when it went. The selection, the current clip and track, the group and the insert menu follow; what went with
    /// its track goes, and a gesture under way ends.
    pub(crate) fn rekey(&mut self, track: impl Fn(usize, usize) -> Option<(usize, usize)>) {
        let map = |r: ClipRef| {
            track(r.channel, r.lane).map(|(channel, lane)| ClipRef { channel, lane, ..r })
        };
        (self.touch, self.drag, self.marquee) = (None, None, None);
        match self.selected.map(map) {
            Some(Some(r)) => self.selected = Some(r),
            Some(None) => self.deselect(),
            None => {}
        }
        self.pick = (self.pick).and_then(|(r, i)| map(r).map(|r| (r, i)));
        self.track = self.track.and_then(|(c, l)| track(c, l));
        self.group = std::mem::take(&mut self.group)
            .into_iter()
            .filter_map(map)
            .collect();
        self.insert = (self.insert).and_then(|(c, l, tick)| track(c, l).map(|(c, l)| (c, l, tick)));
        self.cursor = (self.cursor).and_then(|(tick, (c, l))| track(c, l).map(|t| (tick, t)));
        // A clip whose track is gone goes from the clipboard.
        let kept = std::mem::take(&mut self.clipboard).into_iter();
        self.clipboard = kept.filter_map(|(r, k, c)| Some((map(r)?, k, c))).collect();
    }

    /// The view scrolled or zoomed sideways: the insert menu hides. (The floating row hides at any view change by
    /// itself.)
    pub(crate) fn view_moved(&mut self) {
        self.insert = None;
    }

    /// A touch on empty track space held past a tap, or its marquee: the playlist does not scroll under it (`now`,
    /// input time).
    pub(crate) fn busy(&self, now: f64) -> bool {
        self.marquee.is_some()
            || (self.touch).is_some_and(|t| {
                matches!(t.on, Target::Lane(..)) && gesture::is_long(now - t.since)
            })
    }

    /// Clears the selection and what goes with it; the current clip and track, the double-tap timer, the cursor and the
    /// clipboard stay.
    fn deselect(&mut self) {
        *self = Self {
            track: self.track,
            last_tap: self.last_tap,
            cursor: self.cursor,
            clipboard: std::mem::take(&mut self.clipboard),
            ..Self::default()
        };
    }

    /// The keymap's editing actions while the playlist holds the keyboard (`ui/edit_keys.rs`), on the selected clip or
    /// the group as the floating row's: `copy` puts them on the clip clipboard, `cut` too, `cut` and `delete` remove
    /// them, `paste` adds the clipboard at the grid cursor ([`paste_clips`]), `open` selects the clip at the cursor
    /// (`Select clip`, no edit) or else is `Add clip` there (`New Clip`'s rule, note and drum tracks only),
    /// `close` deselects; `selectAll` is the piano roll's only. Each edit is one final edit; each action hides the
    /// floating row and the insert menu. Called after the playlist's popups, which take Escape first; nothing during a
    /// touch. `snap` is the grid in ticks.
    pub(crate) fn keys(&mut self, ui: &egui::Ui, app: &mut AppCtx, snap: f64) {
        if self.touch.is_some() || self.drag.is_some() || self.marquee.is_some() {
            return;
        }
        let ctx = ui.ctx();
        let Some(k) = edit_keys::take(ctx) else {
            return;
        };
        self.row.hide();
        // A key shows the selection's handles, as before a move left it quiet.
        (self.insert, self.quiet) = (None, false);
        ctx.request_repaint();
        let targets = self.targets();
        if matches!(k, EditKey::Copy | EditKey::Cut) && !targets.is_empty() {
            self.copy_selection(app.song);
        }
        let (tick, (channel, lane)) = self.cursor.unwrap_or((0.0, self.current_track(app)));
        match k {
            EditKey::Close => self.deselect(),
            EditKey::Copy | EditKey::SelectAll => {}
            EditKey::Cut | EditKey::Delete if !targets.is_empty() => {
                self.delete(app, &targets);
                // A cut's clips are to paste: the question would drop them from the clipboard with their tracks.
                if k == EditKey::Cut {
                    self.emptied = None;
                }
            }
            EditKey::Cut | EditKey::Delete => {}
            EditKey::Paste => {
                let show = app.settings.show_hidden_tracks;
                let pasted =
                    paste_clips(app.song, &self.clipboard, tick, (channel, lane), snap, show);
                if !pasted.is_empty() {
                    self.set_selection(pasted);
                    app.song_edited(true);
                }
            }
            EditKey::Open => {
                let bar = app.song.time_sig.bar_beats();
                let takes = |l: &Lane| matches!(l.kind, LaneKind::Notes | LaneKind::Drums);
                // `Select clip` on a clip at the cursor, else `Add clip`.
                let spans = |c: &&Clip| f64::from(c.start_tick) <= tick && tick < end_tick(c);
                let on = (app.song.channels.get(channel))
                    .and_then(|c| c.lanes.get(lane))
                    .and_then(|l| l.clips.iter().find(spans));
                if let Some(clip) = on {
                    let r = ClipRef {
                        channel,
                        lane,
                        id: clip.id,
                    };
                    (self.selected, self.group) = (Some(r), Vec::new());
                    // As a tap on it: its track becomes current too, so the open editor follows it.
                    self.make_current(app.song, app.selected, r);
                } else if let Some(l) = lane_mut(app.song, channel, lane).filter(|l| takes(l)) {
                    let id = add_clip(l, tick, snap, bar);
                    let r = ClipRef { channel, lane, id };
                    (self.selected, self.group) = (Some(r), Vec::new());
                    self.make_current(app.song, app.selected, r);
                    app.song_edited(true);
                }
            }
        }
    }

    /// A tap on clip `r` (module docs): it becomes current and selected, or deselected when it was both already; on
    /// the current clip < 0.3 s after the previous clip tap it opens its editor instead.
    fn tap_clip(&mut self, app: &mut AppCtx, r: ClipRef, touch: &Touch, now: f64) -> Option<Nav> {
        let double = self.current_clip(app) == Some(r) && now - self.last_tap < DOUBLE_TAP_S;
        self.last_tap = now;
        if double {
            self.selected = touch.prior;
            return Some(r.edit());
        }
        self.make_current(app.song, app.selected, r);
        // Current and selected with handles already: the tap deselects. Else it is selected with them (also out of a
        // quiet selection).
        self.selected = (!(touch.current && touch.shown && touch.prior == Some(r))).then_some(r);
        self.quiet = false;
        None
    }

    /// Selects `clips`: one is the selected clip, with its handles; several are the group.
    fn set_selection(&mut self, clips: Vec<ClipRef>) {
        self.quiet = false;
        (self.selected, self.group) = match clips.len() {
            1 => (clips.first().copied(), Vec::new()),
            _ => (None, clips),
        };
    }

    /// What the floating buttons act on: the group, or the selected clip.
    fn targets(&self) -> Vec<ClipRef> {
        if self.group.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.group.clone()
        }
    }

    /// One frame of `touch` with the pointer at `pos` at time `now`, `down` while the button is: a drag once it leaves
    /// the calm zone (a clip that was not current only after 300 ms; before then the move cancels it), or at the lift
    /// a tap or a long press. Returns the editor a double tap opens.
    fn decide(
        &mut self,
        app: &mut AppCtx,
        mut touch: Touch,
        (pos, now, down): (Option<Pos2>, f64, bool),
        rows: &[(usize, usize, Rect)],
        (axis, snap): (TimeAxis, f64),
    ) -> Option<Nav> {
        let euclid = gesture::left_calm(touch.origin, pos, app.cx.m.u(1.0));
        // Empty lane space is the scroller's: its per-axis calm zone decides.
        let moved = match touch.on {
            Target::Lane(..) => self.left_calm.unwrap_or(euclid),
            _ => euclid,
        };
        let tap = !gesture::is_long(now - touch.since);
        match touch.on {
            Target::Clip(_, part) if moved && !touch.cancelled => {
                if touch.current || !tap {
                    self.start(app, part, Some(touch.origin), axis);
                } else {
                    // Moved before a clip that is not current could drag: the touch is cancelled, the selection hidden.
                    touch.cancelled = true;
                    self.selected = None;
                    self.touch = Some(touch);
                }
            }
            // A touch in the group's frame moves it at once (as the current clip does).
            Target::Group(_, part) if moved => self.start_group(app, part, touch.origin, axis),
            // Moved within 300 ms on empty space: a drag (the playlist's own), never a marquee.
            Target::Lane(..) if moved && tap => {}
            // Held on empty space: the marquee, which follows the finger from this frame on.
            Target::Lane(..) if down && !tap => {
                self.marquee = Some(Marquee::at(touch.origin, false));
            }
            _ if down => self.touch = Some(touch),
            Target::Clip(..) if touch.cancelled => {}
            // A handle's tap is an empty-space tap: it only deselects, a clip's handle or the group's.
            Target::Clip(_, part) | Target::Group(_, part) if tap && part != Part::Body => {
                self.deselect()
            }
            Target::Clip(r, _) if tap => return self.tap_clip(app, r, &touch, now),
            // A long press shows the touched clip's handles and row; the current clip stays.
            Target::Clip(r, _) => (self.selected, self.quiet) = (Some(r), false),
            // A tap on a clip of the group: the group goes, and the clip takes the tap; elsewhere in its frame it only
            // deselects. A long press shows the group.
            Target::Group(Some(r), Part::Body) if tap => {
                self.group.clear();
                touch.prior = None;
                touch.current = self.current_clip(app) == Some(r);
                return self.tap_clip(app, r, &touch, now);
            }
            Target::Group(None, Part::Body) if tap => self.deselect(),
            // A handle's long press keeps the group; a long press on the frame shows it.
            Target::Group(..) => self.quiet = false,
            // On empty lane space a tap only deselects a showing selection; else, on the current track it shows the
            // insert menu at the snapped beat, or hides it when the touch-down hid it there; on another track it makes
            // that track current. A long press there is the menu's too (its marquee's, `Self::show`).
            Target::Lane(channel, lane) if tap && !moved => {
                // A quiet selection too: a selection is there, so the tap only deselects.
                if self.selected.is_some() || !self.group.is_empty() {
                    self.deselect();
                } else if (channel, lane) == self.current_track(app) {
                    let at = (channel, lane, cell(axis.tick(touch.origin.x), snap));
                    if touch.menu != Some(at) && rows.iter().any(|r| (r.0, r.1) == (channel, lane))
                    {
                        self.insert = Some(at);
                    }
                } else {
                    self.make_current_track(app, channel, lane);
                }
            }
            // A long press lifted in place before its marquee started: the insert menu there, as the marquee's would.
            Target::Lane(channel, lane) if !moved => {
                self.deselect();
                self.insert = Some((channel, lane, cell(axis.tick(touch.origin.x), snap)));
            }
            Target::Lane(..) => {}
        }
        None
    }

    /// The insert menu while it shows ([`insert::show`]): `New Clip` and `Paste` over its track's row at its snapped
    /// tick, the insert cursor across the row. `New Clip` adds a one-bar clip there on a note or drum track
    /// ([`add_clip`]), the current, selected clip; on an audio track it opens the sample picker, whose first pick makes
    /// the clip there. An automation track takes no new clip, so there it is dimmed. `Paste` pastes the clip clipboard
    /// there ([`paste_clips`]), selecting the copies; it is dimmed while the clipboard is empty. Any other press hides
    /// the menu, as does a track that went. Returns whether this frame's press is on it, and the picker to open.
    #[allow(clippy::too_many_arguments)]
    fn insert_menu(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        grid: Rect,
        rows: &[(usize, usize, Rect)],
        axis: TimeAxis,
        (snap, bar): (f64, u32),
        pressed: bool,
    ) -> (bool, Option<Nav>) {
        let Some((channel, lane, tick)) = self.insert else {
            return (false, None);
        };
        let Some(&(.., row)) = rows.iter().find(|r| (r.0, r.1) == (channel, lane)) else {
            self.insert = None;
            return (false, None);
        };
        let kind = (app.song.channels.get(channel))
            .and_then(|c| c.lanes.get(lane))
            .map(|l| l.kind);
        let new = matches!(
            kind,
            Some(LaneKind::Notes | LaneKind::Drums | LaneKind::Audio)
        );
        let items = [("New Clip", new), ("Paste", !self.clipboard.is_empty())];
        let at = (axis.x(tick), row.y_range(), row.y_range());
        let tap = insert::show(ui, &app.cx, Id::new("clip_insert"), grid, at, &items);
        let mut nav = None;
        if tap.is_some() {
            self.insert = None;
        }
        match tap {
            Some(Tap::Button(0)) if kind == Some(LaneKind::Audio) => {
                let tick = tick as u32;
                nav = Some(Nav::Open(Overlay::TrackSamplePicker {
                    channel,
                    lane,
                    tick,
                }));
            }
            Some(Tap::Button(0)) => {
                if let Some(l) = lane_mut(app.song, channel, lane) {
                    let id = add_clip(l, tick, snap, bar);
                    let r = ClipRef { channel, lane, id };
                    (self.selected, self.quiet) = (Some(r), false);
                    self.make_current(app.song, app.selected, r);
                    app.song_edited(true);
                }
            }
            Some(Tap::Button(_)) => {
                let show = app.settings.show_hidden_tracks;
                let pasted =
                    paste_clips(app.song, &self.clipboard, tick, (channel, lane), snap, show);
                if !pasted.is_empty() {
                    self.set_selection(pasted);
                    app.song_edited(true);
                }
            }
            Some(Tap::Away) | None => {}
        }
        (pressed && tap != Some(Tap::Away), nav)
    }

    /// Starts dragging the selected clip's `part` from the press at `press`.
    fn start(&mut self, app: &AppCtx, part: Part, press: Option<Pos2>, axis: TimeAxis) {
        let clip = self
            .selected
            .zip(self.selected.and_then(|r| find(app.song, r)));
        if let (Some((r, clip)), Some(press)) = (clip, press) {
            self.drag = Some(Drag {
                part,
                clips: vec![(r, Span::of(clip), Span::of(clip))],
                from: axis.tick(press.x),
                edited: false,
                y: press.y,
                rows_down: 0,
                visit: None,
                before: None,
            });
        }
    }

    /// Starts dragging the group's `part` (its frame moves it, a handle resizes each clip) from the press at `press`.
    fn start_group(&mut self, app: &AppCtx, part: Part, press: Pos2, axis: TimeAxis) {
        let clips = (self.group.iter())
            .filter_map(|&r| find(app.song, r).map(|c| (r, Span::of(c), Span::of(c))))
            .collect();
        self.drag = Some(Drag {
            part,
            clips,
            from: axis.tick(press.x),
            edited: false,
            y: press.y,
            rows_down: 0,
            visit: None,
            before: None,
        });
    }

    /// Moves the dragged part to the pointer, an ongoing edit, and ends the drag with a final edit when the
    /// button is released. A clip changed from outside ends the drag where it is. A pinch (two touches) only zooms
    /// the playlist: it ends the drag with the clips put back. A group moves as one: its first clip snaps as it would
    /// alone, the others keep their places relative to it, none before the song start. A body drag also moves the clips
    /// between the `rows` ([`Self::drag_rows`]).
    fn follow_drag(
        &mut self,
        ui: &egui::Ui,
        app: &mut AppCtx,
        axis: TimeAxis,
        snap: f64,
        rows: &[(usize, usize, Rect)],
    ) {
        // Every frame, for its latch: a press held over a focus loss is up, and moves nothing while it lasts.
        let down = primary_down_live(ui, Id::new("clip_drag"));
        // Unlike `latest_pos`, kept through a `PointerGone` in the release frame (a touch's lift).
        let pos = ui.input(|i| {
            i.pointer
                .interact_pos()
                .filter(|_| down || !i.pointer.primary_down())
        });
        let Some(mut drag) = self.drag.take() else {
            return;
        };
        if drag.clips.is_empty() || !drag.unchanged(app.song) {
            return;
        }
        if ui.input(|i| i.multi_touch().is_some()) {
            if drag.edited {
                self.drag_rows(app, &mut drag, 0);
                for &(r, orig, _) in &drag.clips {
                    if let Some(clip) = find_mut(app.song, r) {
                        orig.apply(clip);
                    }
                }
                app.song_edited(true);
            }
            return;
        }
        // The rows first: the spans below go to the clips where they are now.
        let mut moved = false;
        if let (Part::Body, Some(pos)) = (drag.part, pos) {
            let tracks = track_rows(app.song, app.settings.show_hidden_tracks);
            let index = |y: f32| {
                let row = rows.iter().min_by(|a, b| {
                    let d = |r: &&(usize, usize, Rect)| (r.2.center().y - y).abs();
                    d(a).total_cmp(&d(b))
                })?;
                (tracks.iter()).position(|t| (t.channel, t.lane) == (row.0, row.1))
            };
            if let (Some(to), Some(from)) = (index(pos.y), index(drag.y)) {
                moved = self.drag_rows(app, &mut drag, to as isize - from as isize);
            }
        }
        // A handle leaves a clip at least half a step long (1/(2d) beat).
        let min = f64::from(PPQ) / (2.0 * f64::from(app.song.time_sig.steps_per_beat()));
        let spans: Vec<Span> = match pos {
            None => drag.clips.iter().map(|c| c.2).collect(),
            Some(pos) => {
                let delta = axis.tick(pos.x) - drag.from;
                let each = |&(r, orig, _): &(ClipRef, Span, Span)| {
                    let loop_beats = find(app.song, r).map_or(0.0, |c| c.loop_len_beats);
                    dragged(orig, loop_beats, drag.part, delta, snap, min)
                };
                if drag.part != Part::Body || drag.clips.len() == 1 {
                    // A handle resizes each clip by its own law (for a group).
                    drag.clips.iter().map(each).collect()
                } else {
                    // A group moves as one: as its first clip would alone, never before the song start.
                    let (first, o0) = (each(&drag.clips[0]), drag.clips[0].1);
                    let lowest = drag.clips.iter().map(|c| i64::from(c.1.start)).min();
                    let d =
                        (i64::from(first.start) - i64::from(o0.start)).max(-lowest.unwrap_or(0));
                    (drag.clips.iter())
                        .map(|&(_, orig, _)| Span {
                            start: u32::try_from(i64::from(orig.start) + d).unwrap_or(0),
                            ..orig
                        })
                        .collect()
                }
            }
        };
        let mut clips = drag.clips.clone();
        for ((r, _, last), span) in clips.iter_mut().zip(spans) {
            if span != *last
                && let Some(clip) = find_mut(app.song, *r)
            {
                span.apply(clip);
                *last = span;
                moved = true;
            }
        }
        if moved {
            app.song_edited(false);
        }
        let edited = drag.edited || moved;
        if down {
            self.drag = Some(Drag {
                clips,
                edited,
                ..drag
            });
        } else if edited {
            Drag { clips, ..drag }.scale_sample(app.song, &mut app.audio.samples, app.files);
            app.song_edited(true);
        }
    }

    /// Puts the dragged clips `want` rows down the track list from where the drag found them (clips drag between tracks
    /// of any channel by lane kind), each on a track that takes it ([`takes`]): a lone clip on the one nearest that row
    /// (the nearer its own on a tie), a group only when every clip's row takes it, else back on its own rows. The
    /// current clip stays current and its new track becomes the current one; the selection follows. Whether a clip
    /// moved.
    // ponytail: the lone clip's nearest track may be scrolled out; restrict it to the shown rows if that shows.
    fn drag_rows(&mut self, app: &mut AppCtx, drag: &mut Drag, want: isize) -> bool {
        let tracks = track_rows(app.song, app.settings.show_hidden_tracks);
        let index =
            |r: ClipRef| (tracks.iter()).position(|t| (t.channel, t.lane) == (r.channel, r.lane));
        let homes: Option<Vec<(ClipRef, usize, usize)>> = (drag.clips.iter())
            .map(|&(r, ..)| {
                let at = index(r)?;
                Some((r, at, at.checked_add_signed(-drag.rows_down)?))
            })
            .collect();
        let Some(homes) = homes else {
            return false;
        };
        let fits = |r: ClipRef, i: usize| {
            (tracks.get(i)).is_some_and(|t| takes(app.song, r, (t.channel, t.lane)))
        };
        let to: Vec<usize> = match homes[..] {
            [(r, at, home)] => {
                let want = home.saturating_add_signed(want);
                let near = (0..tracks.len()).filter(|&i| fits(r, i));
                vec![
                    near.min_by_key(|&i| (i.abs_diff(want), i.abs_diff(home)))
                        .unwrap_or(at),
                ]
            }
            _ => (homes.iter())
                .map(|&(r, _, home)| home.checked_add_signed(want).filter(|&i| fits(r, i)))
                .collect::<Option<_>>()
                .unwrap_or_else(|| homes.iter().map(|h| h.2).collect()),
        };
        let moves: Vec<(ClipRef, (usize, usize))> = (homes.iter().zip(&to))
            .filter(|(h, i)| h.1 != **i)
            .map(|(h, &i)| (h.0, (tracks[i].channel, tracks[i].lane)))
            .collect();
        if moves.is_empty() {
            return false;
        }
        drag.rows_down = to[0] as isize - homes[0].2 as isize;
        let current = self.current_clip(app);
        let pairs: Vec<(ClipRef, ClipRef)> = match drag.before.take() {
            Some((refs, lanes)) if drag.rows_down == 0 => {
                for (c, l, lane) in lanes {
                    if let Some(to) = lane_mut(app.song, c, l) {
                        *to = lane;
                    }
                }
                drag.visit = None;
                homes.iter().map(|h| h.0).zip(refs).collect()
            }
            before => {
                let refs = || homes.iter().map(|h| h.0).collect();
                let (refs, mut lanes) = before.unwrap_or_else(|| (refs(), Vec::new()));
                for (c, l) in moves.iter().flat_map(|&(r, to)| [(r.channel, r.lane), to]) {
                    if !lanes.iter().any(|s| (s.0, s.1) == (c, l))
                        && let Some(lane) = app.song.channels.get(c).and_then(|ch| ch.lanes.get(l))
                    {
                        lanes.push((c, l, lane.clone()));
                    }
                }
                drag.before = Some((refs, lanes));
                let moved = move_clips(app.song, &moves);
                moves.iter().map(|m| m.0).zip(moved).collect()
            }
        };
        // One lookup each, so that clips trading tracks follow their own.
        let map = |r: ClipRef| pairs.iter().find(|p| p.0 == r).map_or(r, |p| p.1);
        for c in &mut drag.clips {
            // A clip back from `before` is where that left it.
            c.0 = map(c.0);
            c.2 = find(app.song, c.0).map_or(c.2, Span::of);
        }
        self.group.iter_mut().for_each(|r| *r = map(*r));
        self.selected = self.selected.map(map);
        self.pick = self.pick.map(|(r, i)| (map(r), i));
        if let Some(new) = current.map(map).filter(|&r| Some(r) != current) {
            if let Some((c, l, id)) = drag.visit.take()
                && let Some(lane) = lane_mut(app.song, c, l)
            {
                lane.file.current_clip_id = id;
            }
            let lane = lane_mut(app.song, new.channel, new.lane);
            drag.visit = lane.map(|l| (new.channel, new.lane, l.file.current_clip_id));
            self.make_current(app.song, app.selected, new);
        }
        true
    }

    /// The clips a drag under way has edited, each as the drag found it and where it is now (a clip moved to another
    /// track has another [`ClipRef`] there). Empty until the drag edits.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn dragging(&self) -> Vec<(ClipRef, ClipRef)> {
        let Some(drag) = self.drag.as_ref().filter(|d| d.edited) else {
            return Vec::new();
        };
        let now = drag.clips.iter().map(|c| c.0);
        match &drag.before {
            Some((found, _)) => found.iter().copied().zip(now).collect(),
            None => now.map(|r| (r, r)).collect(),
        }
    }

    /// The playlist goes under a drag it will not see end (`app::left_view`: the mixer shows): the drag ends as its
    /// release would, the clips where it last left them. Whether that is a final edit to make.
    pub(crate) fn end_drag(
        &mut self,
        song: &mut Song,
        samples: &mut SampleCache,
        files: &ProjectFiles,
    ) -> bool {
        let Some(drag) = self.drag.take().filter(|d| d.edited && d.unchanged(song)) else {
            return false;
        };
        drag.scale_sample(song, samples, files);
        true
    }

    /// Deletes the clips `targets` and deselects, one final edit, noting what that left empty ([`emptied`]).
    fn delete(&mut self, app: &mut AppCtx, targets: &[ClipRef]) {
        for r in targets {
            if let Some(lane) = lane_mut(app.song, r.channel, r.lane) {
                delete_clip(lane, r.id);
            }
        }
        self.deselect();
        self.emptied = emptied(app.song, targets);
        app.song_edited(true);
    }

    fn copy_selection(&mut self, song: &Song) {
        self.clipboard = self
            .targets()
            .iter()
            .filter_map(|&r| {
                let kind = song.channels.get(r.channel)?.lanes.get(r.lane)?.kind;
                Some((r, kind, find(song, r)?.clone()))
            })
            .collect();
    }

    /// Floating button `i`, at `center`, clicked over the selected clip or the group.
    fn act(&mut self, app: &mut AppCtx, i: usize, center: Pos2, snap: f64) -> Option<Nav> {
        let targets = self.targets();
        let &first = targets.first()?;
        match OPS[i].0 {
            ClipOp::Copy => {
                self.copy_selection(app.song);
                let copies = copy_clips(app.song, &targets, snap);
                self.set_selection(copies);
                app.song_edited(true);
            }
            ClipOp::Delete => self.delete(app, &targets),
            ClipOp::Snap => {
                let mut changed = false;
                for r in targets {
                    if let Some(clip) = find_mut(app.song, r) {
                        let before = clip.start_tick;
                        quantize(clip, snap);
                        changed |= clip.start_tick != before;
                    }
                }
                if changed {
                    app.song_edited(true);
                }
            }
            // The group's `Edit` opens its first clip's editor.
            ClipOp::Edit => return Some(first.edit()),
            ClipOp::More => self.more = Some(center),
        }
        None
    }

    /// The `More...` dropdown while it is open, for the selected clip or the group.
    fn more_menu(&mut self, ui: &egui::Ui, app: &mut AppCtx) {
        let targets = self.targets();
        let (Some(center), false) = (self.more, targets.is_empty()) else {
            self.more = None;
            return;
        };
        let playhead = app.snapshot.map_or(0, |s| s.transport_tick);
        let clips: Vec<&Clip> = targets.iter().filter_map(|&r| find(app.song, r)).collect();
        let items = more_items(&clips, playhead);
        let result = popup::show(
            ui.ctx(),
            Id::new("clip_more"),
            center,
            app.regions.screen,
            &items,
            &app.cx,
        );
        if result.close {
            self.more = None;
            // Nothing else redraws the playlist without the popup.
            ui.ctx().request_repaint();
        }
        let mut edited = false;
        for r in targets {
            let Some(lane) = lane_mut(app.song, r.channel, r.lane) else {
                continue;
            };
            let clip = lane.clips.iter_mut().find(|c| c.id == r.id);
            edited |= match (result.selected, clip) {
                (Some(MoreOp::Slice), _) => slice(lane, r.id, playhead).is_some(),
                (Some(MoreOp::UnlinkLoop), Some(clip)) => unlink_loop(clip),
                (Some(op @ (MoreOp::Mute | MoreOp::Unmute)), Some(clip)) => {
                    let muted = op == MoreOp::Mute;
                    let changed = clip.muted != muted;
                    clip.muted = muted;
                    changed
                }
                _ => false,
            };
        }
        if edited {
            app.song_edited(true);
        }
    }
}

/// The track list's scroll, as the screen y where its first row (the master's, scrolled out or not) starts: the
/// first of the visible `rows` less the rows above it. A scroll by whole rows moves it too, and a selection that
/// moves between rows does not. `px` is the device pixel of gap that the rows' rects leave out under each row; with
/// `show_hidden` the hidden tracks are rows ([`track_rows`]).
fn list_top(song: &Song, rows: &[(usize, usize, Rect)], px: f32, show_hidden: bool) -> f32 {
    let Some(&(channel, lane, first)) = rows.first() else {
        return 0.0;
    };
    let above = (track_rows(song, show_hidden).iter())
        .take_while(|t| (t.channel, t.lane) != (channel, lane))
        .count();
    first.top() - above as f32 * pitch(rows, px)
}

/// The rows' pitch: the lane rects leave out the device pixel `px` of gap under each row.
fn pitch(rows: &[(usize, usize, Rect)], px: f32) -> f32 {
    match rows {
        [a, b, ..] => b.2.top() - a.2.top(),
        [a] => a.2.height() + px,
        [] => 0.0,
    }
}

/// The selection box of the clips `sel`: their earliest start to their latest end, from the top of the first selected
/// track row to the bottom of the last (Y_r to Y_{r+1}, on screen or scrolled out), by the playlist's scroll and zoom;
/// no padding and no clip insets. `None` for no clip on a shown track.
fn selection_box(
    song: &Song,
    sel: &[ClipRef],
    rows: &[(usize, usize, Rect)],
    axis: TimeAxis,
    px: f32,
    show_hidden: bool,
) -> Option<Rect> {
    let tracks = track_rows(song, show_hidden);
    let (top, pitch) = (list_top(song, rows, px, show_hidden), pitch(rows, px));
    (sel.iter())
        .filter_map(|&r| {
            let clip = find(song, r)?;
            let i = (tracks.iter()).position(|t| (t.channel, t.lane) == (r.channel, r.lane))?;
            let y = top + i as f32 * pitch;
            let x = axis.x(f64::from(clip.start_tick))..=axis.x(end_tick(clip));
            Some(Rect::from_x_y_ranges(x, y..=y + pitch))
        })
        .reduce(Rect::union)
        .filter(|_| !rows.is_empty())
}

/// The selection box `b`'s handles as drawn: the start capsule ending 0.5 U inside its left edge (none when it would
/// touch the end one), the end capsule from 0.15 U right of its right edge, both centred on it vertically; with
/// `scale`, the third handle's bar centred on its right edge, 0.12 U below it.
fn handle_rects(b: Rect, scale: bool, m: &Metrics) -> Vec<(Part, Rect)> {
    let end = end_handle(b, m);
    let size = capsule(m);
    let start = Rect::from_min_size(pos2(b.left() + m.u(START_IN) - size.x, end.top()), size);
    let mut handles = Vec::new();
    if start.right() < end.left() {
        handles.push((Part::Start, start));
    }
    handles.push((Part::End, end));
    if scale {
        let bar = vec2(m.u(SCALE_BAR[0]), m.u(SCALE_BAR[1]) + 2.0 / m.ppp());
        let top = b.bottom() + m.u(SCALE_OFF);
        let bar = Rect::from_min_size(pos2(b.right() - bar.x / 2.0, top), bar);
        handles.push((Part::Scale, bar));
    }
    handles
}

/// A selection handle's size: (0.42 U + 2 px) × (1.41 U + 2 px).
pub(crate) fn capsule(m: &Metrics) -> Vec2 {
    let px2 = 2.0 / m.ppp();
    vec2(m.u(HANDLE[0]) + px2, m.u(HANDLE[1]) + px2)
}

/// The end handle of the box `b` (a selection, a note, a hit): the capsule from 0.15 U right of it, centred on it
/// vertically.
pub(crate) fn end_handle(b: Rect, m: &Metrics) -> Rect {
    let size = capsule(m);
    Rect::from_min_size(
        pos2(b.right() + m.u(END_OFF), b.center().y - size.y / 2.0),
        size,
    )
}

/// Its touch zone, centred on it: as tall as a capsule and [`ZONE`] wide (the start one at most a third of the
/// selection box `b`), or for the bar as wide as it and [`ZONE`] tall.
fn handle_zone(part: Part, rect: Rect, b: Rect, m: &Metrics) -> Rect {
    let size = match part {
        Part::Start => vec2(m.u(ZONE).min(b.width() / 3.0), rect.height()),
        Part::Scale => vec2(rect.width(), m.u(ZONE)),
        Part::End | Part::Body => vec2(m.u(ZONE), rect.height()),
    };
    Rect::from_center_size(rect.center(), size)
}

/// Fills the selection box `b` (corners `corner`) with `text` at α 0.15 + 0.05·cos(2π·p) and advances p (`pulse`)
/// 0.0125 a drawn frame, wrapping at 1 (an 80-frame cycle); without a box p restarts at 0.
pub(crate) fn paint_pulse(
    painter: &Painter,
    cx: &UiCtx,
    b: Option<Rect>,
    corner: f32,
    pulse: &mut f32,
) {
    let Some(b) = b else {
        *pulse = 0.0;
        return;
    };
    let a = PULSE[0] + PULSE[1] * (std::f32::consts::TAU * *pulse).cos();
    let text = cx.theme.colors.text;
    painter.rect_filled(b, corner, Rgba { a, ..text }.to_color32());
    *pulse = (*pulse + PULSE[2]).fract();
    painter.ctx().request_repaint();
}

/// The clip editors' selection handles on the box `b` (piano roll, drum note grid, automation), each with its touch
/// zone (at least 1 U × 1 U, centred): with `end` the end capsule, its edges on whole device pixels; with `scale` the
/// ↔ bar, its right edge on the box's, U + 2 px × 0.56 U + 2 px, 0.12 U below the box.
pub(crate) fn editor_handles(
    b: Rect,
    end: bool,
    scale: bool,
    m: &Metrics,
) -> Vec<(Part, Rect, Rect)> {
    let mut handles = Vec::new();
    if end {
        let r = end_handle(b, m);
        let floor = |p: Pos2| pos2(m.floor_px(p.x), m.floor_px(p.y));
        let r = Rect::from_min_max(floor(r.min), floor(r.max));
        handles.push((Part::End, r, handle_zone(Part::End, r, b, m)));
    }
    if scale {
        let px2 = 2.0 / m.ppp();
        let size = vec2(m.u(1.0) + px2, m.u(SCALE_BAR[1]) + px2);
        let r = Rect::from_min_size(pos2(b.right() - size.x, b.bottom() + m.u(SCALE_OFF)), size);
        handles.push((Part::Scale, r, handle_zone(Part::Scale, r, b, m)));
    }
    handles
}

/// The press growth and release wobble of the clip editors' selection handles ([`press_growth`]).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HandlePress {
    held: Option<(Part, f64)>,
    released: Option<(Part, f64, f32)>,
}

impl HandlePress {
    /// Follows the handle a gesture holds and since when: one let go wobbles from the growth it had.
    pub(crate) fn follow(&mut self, held: Option<(Part, f64)>, now: f64) {
        if held.is_none()
            && let Some((part, t)) = self.held
        {
            self.released = Some((part, now, grow(now - t)));
        }
        self.held = held;
    }

    /// Handle `part`'s growth now, in device pixels; asks for the next frame while it moves.
    pub(crate) fn growth(&self, ctx: &egui::Context, part: Part, now: f64) -> f32 {
        let pressed = self.held.filter(|h| h.0 == part).map(|h| h.1);
        let released = self.released.filter(|r| r.0 == part).map(|r| (r.1, r.2));
        let (growth, moving) = press_growth(pressed, released, now);
        if moving {
            ctx.request_repaint();
        }
        growth
    }
}

/// Whether a clip of `lane` alone in the selection takes the third handle: an audio clip that does not loop,
/// its length at most its loop less its content offset (wrapped into the loop).
fn scalable(lane: &Lane, clip: &Clip) -> bool {
    let (lp, visible) = (clip.loop_len_beats, Span::of(clip).visible);
    lane.kind == LaneKind::Audio && visible <= lp - wrap(clip.offset_beats, lp)
}

/// A selection handle in `rect` (every editor's): a `menuFill` α 0.3 frame a device pixel wide around a `text` body,
/// both with round ends; a capsule ([`capsule`]) has a `menuFill` α 0.7 grip bar (0.18 × 0.44 of it) in the middle.
/// The third handle's bar (`arrows`) has corners of 0.3 × the body's height h and a `↔` in 0.3·`text` + 0.7·`menuFill`,
/// centred vertically: a shaft 0.12 h tall from 0.62 h inside each end, tips 0.34 h inside them, heads 0.28 h long and
/// 0.5 h tall. Its animation scales it about its centre
/// by `scale` (a pop-in) and widens it by `growth` device pixels (a press, [`press_growth`]); under the mouse it is
/// 5 % larger.
pub(crate) fn paint_handle(
    painter: &Painter,
    cx: &UiCtx,
    rect: Rect,
    (scale, growth): (f32, f32),
    arrows: bool,
) {
    let (m, c) = (cx.m, &cx.theme.colors);
    let px = 1.0 / m.ppp();
    let hover = painter
        .ctx()
        .input(|i| (i.pointer.hover_pos()).filter(|_| !i.pointer.any_down()));
    let k = scale
        * if hover.is_some_and(|p| rect.contains(p)) {
            HOVER
        } else {
            1.0
        };
    let outer = Rect::from_center_size(rect.center(), rect.size() * k).expand(growth * px / 2.0);
    let body = outer.shrink(px);
    if !body.is_positive() {
        return;
    }
    let frame = Rgba {
        a: FRAME_A,
        ..c.menu_fill
    };
    if arrows {
        let h = body.height();
        painter.rect_filled(outer, 0.3 * h + px, frame.to_color32());
        painter.rect_filled(body, 0.3 * h, c.text.to_color32());
        let ink = mix(c.text, c.menu_fill, ARROW_MIX).to_color32();
        let (y, x) = (body.center().y, body.x_range());
        let shaft = Rect::from_x_y_ranges(
            x.min + 0.62 * h..=x.max - 0.62 * h,
            y - 0.06 * h..=y + 0.06 * h,
        );
        painter.rect_filled(shaft, 0.0, ink);
        for (tip, s) in [(x.min + 0.34 * h, 1.0), (x.max - 0.34 * h, -1.0)] {
            let back = tip + s * 0.28 * h;
            let head = vec![
                pos2(tip, y),
                pos2(back, y - 0.25 * h),
                pos2(back, y + 0.25 * h),
            ];
            painter.add(egui::Shape::convex_polygon(head, ink, Stroke::NONE));
        }
    } else {
        let round = |r: Rect| r.width().min(r.height()) / 2.0;
        painter.rect_filled(outer, round(outer), frame.to_color32());
        painter.rect_filled(body, round(body), c.text.to_color32());
        let grip = Rect::from_center_size(outer.center(), outer.size() * Vec2::from(GRIP));
        let ink = Rgba {
            a: GRIP_A,
            ..c.menu_fill
        };
        painter.rect_filled(grip, round(grip), ink.to_color32());
    }
}

/// The third handle's scale of an audio clip by `k`, its sample `frames` long at `rate` (as the audio editor's stretch
/// handle, `Action::Stretch`): the sample's length L, its STR1 stretch × its length at the song tempo, becomes k·L
/// (clamped as the editor clamps it), with the STR1 ratio for that and SYNC on; the clip keeps the visible length the
/// drag gave it. Whether the song changed.
fn stretch_clip(song: &mut Song, r: ClipRef, (frames, rate): (usize, u32), k: f64) -> bool {
    let tempo = song.tempo_bpm;
    let Some(clip) = find(song, r) else {
        return false;
    };
    let (visible, slot) = (clip.visible_len_beats, clip.file.sample.as_deref());
    let Some(stretch) = slot.map(stretch_of) else {
        return false;
    };
    let src = frames as f64 * tempo / (60.0 * f64::from(rate));
    let [lo, hi] = audio_editor::STRETCH;
    let len = (stretch * src * k).clamp(lo * src, hi * src);
    let ratio = stretch_ratio(len, rate, tempo, frames);
    let stretched = Action::Stretch {
        len_beats: len,
        ratio,
    };
    let changed = audio_editor::apply(song, (r.channel, r.lane, r.id), &stretched);
    if let Some(clip) = find_mut(song, r) {
        clip.visible_len_beats = visible;
    }
    changed
}

/// Whether track (`channel`, `lane`) takes clip `r` in a drag: a track of its kind, of any channel (notes play on the
/// new channel's instrument; a drum clip's hits keep their pad indices, a pad the new kit lacks plays nothing); an
/// automation clip only a track of its own channel with its target, as what it automates goes with it.
fn takes(song: &Song, r: ClipRef, (channel, lane): (usize, usize)) -> bool {
    let lane_of = |c: usize, l: usize| song.channels.get(c)?.lanes.get(l);
    let (Some(from), Some(to)) = (lane_of(r.channel, r.lane), lane_of(channel, lane)) else {
        return false;
    };
    from.kind == to.kind
        && (from.kind != LaneKind::Automation || (r.channel == channel && from.target == to.target))
}

/// Moves each clip of `moves` to its track (channel, lane), all out of their tracks first so that clips trading tracks
/// keep their ids: a clip keeps its id where its new track has none with it, else takes the track's next one. Returns
/// the clips as they are now, in order (one whose track is gone stays where it was).
fn move_clips(song: &mut Song, moves: &[(ClipRef, (usize, usize))]) -> Vec<ClipRef> {
    let out: Vec<Option<Clip>> = (moves.iter())
        .map(|&(r, (channel, lane))| {
            lane_mut(song, channel, lane)?;
            let from = lane_mut(song, r.channel, r.lane)?;
            let i = from.clips.iter().position(|c| c.id == r.id)?;
            Some(from.clips.remove(i))
        })
        .collect();
    (moves.iter().zip(out))
        .map(|(&(r, (channel, lane)), clip)| {
            let (Some(mut clip), Some(to)) = (clip, lane_mut(song, channel, lane)) else {
                return r;
            };
            if to.clips.iter().any(|c| c.id == clip.id) {
                to.last_clip_id += 1;
                clip.id = to.last_clip_id;
            }
            to.last_clip_id = to.last_clip_id.max(clip.id);
            let id = clip.id;
            to.clips.push(clip);
            ClipRef { channel, lane, id }
        })
        .collect()
}

fn lane_mut(song: &mut Song, channel: usize, lane: usize) -> Option<&mut Lane> {
    song.channels.get_mut(channel)?.lanes.get_mut(lane)
}

pub(crate) fn find(song: &Song, r: ClipRef) -> Option<&Clip> {
    let lane = song.channels.get(r.channel)?.lanes.get(r.lane)?;
    lane.clips.iter().find(|c| c.id == r.id)
}

fn find_mut(song: &mut Song, r: ClipRef) -> Option<&mut Clip> {
    let lane = lane_mut(song, r.channel, r.lane)?;
    lane.clips.iter_mut().find(|c| c.id == r.id)
}

/// The clip box of a clip whose block ([`clip_rect`]) is `rect`, across a track row less the device pixel of gap under
/// it, Y_r … Y_{r+1} − 1 px: x0 and x1 the block's ends truncated to device pixels, x1 at least t right of x0;
/// y0 = Y_r + 1 px, y1 = Y_{r+1} − t − 1 px.
fn clip_box(rect: Rect, m: &Metrics) -> Rect {
    let x0 = m.floor_px(rect.left());
    Rect::from_min_max(
        pos2(x0, rect.top() + 1.0 / m.ppp()),
        pos2(
            m.floor_px(rect.right()).max(x0 + m.t()),
            rect.bottom() - m.t(),
        ),
    )
}

/// What a clip's drawing takes besides the clip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Look {
    /// The current track's current clip: the light outline.
    current: bool,
    /// Every repeat is dimmed: the clip is muted, or Preview plays another.
    dimmed: bool,
    /// Selected with its handles showing: every repeat but this one is dimmed.
    picked: Option<u64>,
}

impl Look {
    /// Whether repeat `i` is dimmed.
    fn dims(self, i: u64) -> bool {
        self.dimmed || self.picked.is_some_and(|p| p != i)
    }
}

/// A clip of `lane` in its clip box `b` ([`clip_box`]) on `painter`, which is clipped to the grid
/// (a = playlistTrackBackgroundTransparency). Every box has corners of radius 0.15 U:
/// 1. the current clip only: its frame F ([`current_frame`]) filled with `background`, then with `text` at α a, so a
///    ring 1.5t wide shows round the clip on every side; its outline comes after every clip ([`paint_outline`]);
/// 2. the frame: `background` at α 0.3 over the box grown by a device pixel;
/// 3. the body: the channel colour at α a; no header strip;
/// 4. the name ([`clip_name`]): 8 pt in the content colour, in the top 0.7 U from 5 px in (or the grid's left edge)
///    to 2 px before the right end, left-aligned and centred, a middle ellipsis keeping 5;
/// 5. its loop repeats ([`repeats`]): a `background` triangle pointing down at each one's start from the second on,
///    and the miniature of the notes ([`notes`]), automation ([`automation`]) or sample (`wave`, [`waveform`]) in the
///    content rect, 0.7 U below the box's top to 6t above its bottom, in the tint ([`tint`]): notes by velocity, the
///    rest at [`CURVE_TINT`], a dimmed repeat ([`Look::dims`]) at 0. A loop narrower than a pixel draws no repeats.
#[allow(clippy::too_many_arguments)]
fn paint_clip(
    painter: &Painter,
    cx: &UiCtx,
    channel: &Channel,
    lane: &Lane,
    clip: &Clip,
    wave: Option<(&Waveform, Wave)>,
    look: Look,
    b: Rect,
    axis: TimeAxis,
) {
    let UiCtx { theme, m } = *cx;
    let (px, t) = (1.0 / m.ppp(), m.t());
    let (bg, text) = (theme.colors.background, theme.colors.text);
    let a = theme.options.playlist_track_background_transparency;
    let alpha = |c: Rgba, a: f32| Rgba { a, ..c }.to_color32();
    let r = m.u(CORNER);
    if look.current {
        let (lit, lit_r, lit_a) = current_frame(cx, b, look.dimmed);
        painter.rect_filled(lit, lit_r, bg.to_color32());
        painter.rect_filled(lit, lit_r, alpha(text, lit_a));
    }
    painter.rect_filled(b.expand(px), r + px, alpha(bg, FRAME_ALPHA));
    let color = channel_color(channel.hue, theme);
    let body = painter.with_clip_rect(painter.clip_rect().intersect(b));
    body.rect_filled(b, r, alpha(color, a));
    let (content, stops) = tint(theme, color);
    let left = (b.left() + NAME_IN_PX * px).max(painter.clip_rect().left());
    let name = Rect::from_x_y_ranges(
        left..=b.right() - NAME_END_PX * px,
        b.top()..=b.top() + m.u(NAME_H),
    );
    if name.width() > 0.0 {
        let font = m.font_id(FontSize::Pt8);
        let label = clip_name(channel, lane, clip);
        truncated(
            &body,
            name,
            Align2::LEFT_CENTER,
            label,
            font,
            content.to_color32(),
            NAME_K,
        );
    }

    let seen = body.clip_rect();
    let px_ticks = f64::from(px / axis.beat_w) * f64::from(PPQ);
    let Some((base, lp, all)) = repeats(clip, axis.tick(seen.left()), axis.tick(seen.right()))
        .filter(|&(_, lp, _)| lp >= px_ticks)
    else {
        return;
    };
    let x = |i: u64| m.floor_px(axis.x(base + i as f64 * lp)).max(b.left());
    for i in all.clone().filter(|&i| i >= 1) {
        let (x, top) = (x(i), m.floor_px(b.top()));
        let points = vec![
            pos2(x - 2.0 * t, top),
            pos2(x + 2.0 * t, top),
            pos2(x, top + 2.0 * t),
        ];
        body.add(egui::Shape::convex_polygon(
            points,
            bg.to_color32(),
            Stroke::NONE,
        ));
    }
    let area = Rect::from_x_y_ranges(
        b.x_range(),
        b.top() + m.u(NAME_H)..=b.bottom() - CONTENT_END_T * t,
    );
    let steps = lane.kind == LaneKind::Drums && lane.grid == DrumGrid::Steps;
    // The repeats in runs that dim alike: before the picked one, it, and after it.
    let p = look.picked.unwrap_or(all.end).clamp(all.start, all.end);
    for run in [
        all.start..p,
        p..(p + 1).min(all.end),
        (p + 1).min(all.end)..all.end,
    ] {
        if run.is_empty() {
            continue;
        }
        let span = Rect::from_x_y_ranges(x(run.start)..=x(run.end), seen.y_range());
        let painter = body.with_clip_rect(seen.intersect(span));
        let dim = look.dims(run.start);
        let ink = |velocity: u8| {
            let t = if dim {
                0.0
            } else {
                f32::from(velocity) / 255.0
            };
            sample(&stops, t).to_color32()
        };
        notes(&painter, clip, steps, area, axis, ink, m);
        let ink = sample(&stops, if dim { 0.0 } else { CURVE_TINT }).to_color32();
        automation(&painter, clip, area, axis, ink, m);
        if let Some(wave) = wave {
            waveform(&painter, clip, wave, area, axis, ink, m);
        }
    }
}

/// The current clip's frame F around its clip box `b`: grown 1.5t on every side, its corners r + 1.5t, and its α a
/// (a = playlistTrackBackgroundTransparency, × 0.25 when `dimmed`, clamped to 0–1).
fn current_frame(cx: &UiCtx, b: Rect, dimmed: bool) -> (Rect, f32, f32) {
    let g = 1.5 * cx.m.t();
    let a = cx.theme.options.playlist_track_background_transparency;
    let a = if dimmed { 0.25 * a } else { a }.clamp(0.0, 1.0);
    (b.expand(g), cx.m.u(CORNER) + g, a)
}

/// The current clip's outline, drawn after every row and clip so it lies over its neighbours: F's rounded edge
/// ([`current_frame`]) in `text` at α 1 − a² while a² < 0.9, else a, a band t + 1 px wide centred on the edge.
fn paint_outline(painter: &Painter, cx: &UiCtx, b: Rect, dimmed: bool) {
    let (f, r, a) = current_frame(cx, b, dimmed);
    let edge = if a * a < 0.9 { 1.0 - a * a } else { a };
    let color = Rgba {
        a: edge,
        ..cx.theme.colors.text
    };
    let stroke = Stroke::new(cx.m.t() + 1.0 / cx.m.ppp(), color.to_color32());
    painter.rect_stroke(f, r, stroke, StrokeKind::Middle);
}

/// A clip's loop repeats that reach song ticks `from..to`: with L the loop and o the offset wrapped into [0, L),
/// repeat i = 0 … ceil((o + length)/L) − 1 starts at start + i·L − o. Returns repeat 0's start, L and the repeats'
/// range; `None` without a loop.
fn repeats(clip: &Clip, from: f64, to: f64) -> Option<(f64, f64, std::ops::Range<u64>)> {
    let lp = clip.loop_ticks()?;
    let base = f64::from(clip.start_tick) - (clip.offset_beats * f64::from(PPQ)).rem_euclid(lp);
    let n = ((end_tick(clip) - base) / lp).ceil();
    let lo = ((from - base) / lp).floor().clamp(0.0, n);
    let hi = ((to - base) / lp).ceil().clamp(lo, n);
    Some((base, lp, lo as u64..hi as u64))
}

/// The loop repeat of `clip` at song tick `tick` ([`repeats`]), 0 without a loop.
fn repeat_at(clip: &Clip, tick: f64) -> u64 {
    repeats(clip, tick, tick).map_or(0, |(base, lp, _)| {
        ((tick - base) / lp).floor().max(0.0) as u64
    })
}

/// A clip being recorded, whatever its lane (the live take bar), its block `rect` running from its start to the song
/// position across its row less the gap under it; instead of steps 1–5 of [`paint_clip`]: in its clip box `b`
/// ([`clip_box`]), a channel-colour box 1 px in from the box's top and bottom, then a content-colour box 3 px in.
fn paint_take(painter: &Painter, cx: &UiCtx, channel: &Channel, rect: Rect) {
    let (b, px) = (clip_box(rect, cx.m), 1.0 / cx.m.ppp());
    let color = channel_color(channel.hue, cx.theme);
    let (content, _) = tint(cx.theme, color);
    let r = cx.m.u(CORNER);
    painter.rect_filled(b.shrink2(vec2(0.0, px)), r, color.to_color32());
    painter.rect_filled(b.shrink2(vec2(0.0, 3.0 * px)), r, content.to_color32());
}

/// A notes or drums take's clip being recorded, `clip` as it shows live ([`note_record::live`]), its block `rect`
/// running from its start to the song position: issue #26 (a plain recording clip draws only the live
/// bar). The bar ([`paint_take`]), so it still reads as a recording, then over its content-colour box the notes or hits
/// in the channel colour, laid out as the clip's miniature is in [`paint_clip`] (the same content rect), so they stay
/// where they are once the take ends; held ones reach the playhead.
fn paint_live(
    painter: &Painter,
    cx: &UiCtx,
    channel: &Channel,
    lane: &Lane,
    clip: &Clip,
    rect: Rect,
    axis: TimeAxis,
) {
    paint_take(painter, cx, channel, rect);
    let (b, m) = (clip_box(rect, cx.m), cx.m);
    let area = Rect::from_x_y_ranges(
        b.x_range(),
        b.top() + m.u(NAME_H)..=b.bottom() - CONTENT_END_T * m.t(),
    );
    let steps = lane.kind == LaneKind::Drums && lane.grid == DrumGrid::Steps;
    let ink = channel_color(channel.hue, cx.theme).to_color32();
    let body = painter.with_clip_rect(painter.clip_rect().intersect(b));
    notes(&body, clip, steps, area, axis, |_| ink, m);
}

/// A clip's name (DESc@28): an automation lane's custom label, else its auto name (`<Param> : <Module>`, `Level`); an
/// audio lane's clip its sample's name (MAIN@12, the file name); the channel name for any other lane, or when those
/// are empty.
fn clip_name<'a>(channel: &'a Channel, lane: &'a Lane, clip: &'a Clip) -> &'a str {
    let sample = || clip.file.sample.as_deref().map_or("", |s| s.name.as_str());
    let names = match lane.kind {
        LaneKind::Automation => [lane.file.custom_label.as_str(), lane.label.as_str()],
        LaneKind::Audio => [sample(), ""],
        _ => ["", ""],
    };
    names
        .into_iter()
        .find(|n| !n.is_empty())
        .unwrap_or(&channel.name)
}

/// An audio clip's sample with its summary ([`SampleCache::waveform`]), decoded on first sight as the audio editor's
/// is, and where it sits on the content time axis (the editor's [`Wave`]: Speed, a SYNC stretch, REV). `None` for a
/// clip without a sample, a sample that is missing, or one with no playable rate.
fn audio_wave(
    samples: &mut SampleCache,
    files: &ProjectFiles,
    clip: &Clip,
    tempo: f64,
) -> Option<(Waveform, Wave)> {
    let slot = clip.file.sample.as_deref()?;
    let source = samples.waveform(slot, &sample_cache::reader(files))?;
    let (frames, rate) = (source.data.frames(), source.data.sample_rate);
    let wave = Wave::new(
        frames,
        rate,
        tempo,
        slot.params[SPEED],
        stretch_of(slot),
        slot.file.reverse,
    );
    (wave.per_beat > 0.0 && wave.per_beat.is_finite()).then_some((source, wave))
}

/// An audio clip's waveform in `area` in `ink`, by the audio editor's laws (a column a device pixel from min/max peaks,
/// mono centred at h/2 with scale h/2, stereo in halves, a column under a pixel a pixel line, a reversed sample
/// mirrored, Speed and a SYNC stretch in the scale): each loop pass from its start, up to the sample's end or the
/// loop's, as the notes repeat. A loop narrower than a pixel draws nothing.
/// A column reads the sample's summary where it covers whole blocks ([`columns`]), so it costs at most 2·256 frames.
// ponytail: each frame re-scans every column's two partial ends, about 1 ms per channel per visible track; a per-clip
// column cache (keyed by zoom and scroll) or a 16-frame second summary level if that shows up in a profile.
fn waveform(
    painter: &Painter,
    clip: &Clip,
    (source, wave): (&Waveform, Wave),
    area: Rect,
    axis: TimeAxis,
    ink: Color32,
    m: &Metrics,
) {
    let (ppq, px) = (f64::from(PPQ), 1.0 / m.ppp());
    let beats_per_px = f64::from(px / axis.beat_w);
    let Some(lp) = clip.loop_ticks().filter(|&lp| lp >= beats_per_px * ppq) else {
        return;
    };
    // Pass k's content starts at base + k·loop; pass 0 is the one the clip starts in (at its offset).
    let start = f64::from(clip.start_tick);
    let base = start - (clip.offset_beats * ppq).rem_euclid(lp);
    let seen = painter.clip_rect();
    let from = axis.tick(seen.left()).max(start);
    let to = axis.tick(seen.right()).min(end_tick(clip));
    let sound = (wave.end() * ppq).min(lp);
    let rows = wave_rows(area, source.summary.len() > 1);
    let passes =
        ((from - base) / lp).floor().max(0.0) as u64..((to - base) / lp).ceil().max(0.0) as u64;
    for k in passes {
        let at = base + k as f64 * lp;
        let (a, b) = (at.max(from), (at + sound).min(to));
        if b <= a {
            continue;
        }
        let x0 = (axis.x(a) / px).floor() * px;
        let width = ((axis.x(b) - x0) / px).ceil() as usize;
        let left = (axis.tick(x0) - at) / ppq;
        let mut mesh = Mesh::default();
        for ((samples, summary), &row) in source.channels().zip(&rows) {
            let cols = columns(samples, summary, wave, left, beats_per_px, width);
            for (i, peak) in cols.into_iter().enumerate() {
                add_column(&mut mesh, x0 + i as f32 * px, peak, row, px, ink);
            }
        }
        painter.add(mesh);
    }
}

/// `a` moved `t` of the way toward `b`, opaque.
pub(crate) fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let ch = |x: f32, y: f32| x + (y - x) * t;
    Rgba::rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))
}

/// The content colour of a clip in `color` and its miniature's tint stops at 0, [`TINT_MID`] and 1: the channel colour
/// × playlistTrackNoteBrightness (÷ on a theme with a dark text colour); then the background mixed 75 % (25 % on a
/// light theme) toward it, itself, and itself 40 % toward the text colour. Stop 0's alpha is left opaque.
fn tint(theme: &Theme, color: Rgba) -> (Rgba, [Rgba; 3]) {
    let (colors, b) = (&theme.colors, theme.options.playlist_track_note_brightness);
    let max = |c: Rgba| c.r.max(c.g).max(c.b);
    let lightness = |c: Rgba| (max(c) + c.r.min(c.g).min(c.b)) / 2.0;
    let b = if lightness(colors.text) <= 0.25 {
        1.0 / b
    } else {
        b
    };
    let ch = |x: f32| (x * b).min(1.0);
    let content = Rgba::rgb(ch(color.r), ch(color.g), ch(color.b));
    (content, tint_stops(colors, content))
}

/// The tint stops at 0, [`TINT_MID`] and 1 for a `content` colour: the background mixed 75 % (25 % on a light theme)
/// toward it, itself, and itself 40 % toward the text colour.
pub(crate) fn tint_stops(colors: &ThemeColors, content: Rgba) -> [Rgba; 3] {
    let k = if colors.is_dark() { 0.75 } else { 0.25 };
    [
        mix(colors.background, content, k),
        content,
        mix(content, colors.text, 0.4),
    ]
}

/// The tint at `t` in 0..=1, linear between the stops.
pub(crate) fn sample(stops: &[Rgba; 3], t: f32) -> Rgba {
    if t < TINT_MID {
        mix(stops[0], stops[1], t / TINT_MID)
    } else {
        mix(stops[1], stops[2], (t - TINT_MID) / (1.0 - TINT_MID))
    }
}

/// The notes (or drum hits, keyed by pad) the clip plays where the painter's clip rect reaches, every loop pass, as
/// bars t tall in `area`: with lo and hi the content's lowest and highest key and p = the area's height
/// / (hi − lo + 2), a key's bar is at y = bottom − t − (key − lo + 1)·p, from the note's start, max(its length, 2 px)
/// wide (in a step-sequencer drum lane, `steps`, max(2t, a quarter beat)) and cut at its repeat's end, in the `ink` of
/// its velocity.
fn notes(
    painter: &Painter,
    clip: &Clip,
    steps: bool,
    area: Rect,
    axis: TimeAxis,
    ink: impl Fn(u8) -> Color32,
    m: &Metrics,
) {
    let note = |event: &Event| match *event {
        Event::Note {
            len_beats,
            key,
            velocity,
            ..
        } => Some((len_beats, key, velocity)),
        Event::Drum {
            len_beats,
            pad,
            velocity,
            ..
        } => Some((len_beats, pad, velocity)),
        _ => None,
    };
    let (px, t) = (1.0 / m.ppp(), m.t());
    let width = |len: f64| match steps {
        true => (2.0 * t).max(0.25 * axis.beat_w),
        false => (len as f32 * axis.beat_w).max(2.0 * px),
    };
    // Keys from the whole content, so that nothing shifts while the playlist scrolls, and the widest note, in one pass.
    let (lo, hi, widest) = (clip.events.iter().filter_map(note))
        .fold((u8::MAX, u8::MIN, 0.0_f32), |(lo, hi, w), (len, key, _)| {
            (lo.min(key), hi.max(key), w.max(width(len)))
        });
    let Some(lp) = clip.loop_ticks().filter(|_| lo <= hi) else {
        return;
    };
    let seen = painter.clip_rect();
    // A note that starts before the view reaches into it by its width, at most a loop pass: it is cut at its pass's end.
    // A held note of a take outgrows the view as it records, and stays drawn while any of it shows (issue #26).
    let longest = widest.min((lp / f64::from(PPQ)) as f32 * axis.beat_w);
    let start = f64::from(clip.start_tick);
    let from = axis.tick(seen.left() - longest).max(start);
    let to = axis.tick(seen.right()).min(end_tick(clip)).ceil();
    let p = area.height() / (f32::from(hi - lo) + 2.0);
    let base = start - (clip.offset_beats * f64::from(PPQ)).rem_euclid(lp);
    let px_ticks = f64::from(px / axis.beat_w) * f64::from(PPQ);
    // Per key, how far right its drawn notes reach. The plays come in time order, so a note that ends there too lies
    // within one already drawn: a dense clip zoomed out draws a few rects a row, not every note.
    let mut drawn = vec![f32::NEG_INFINITY; usize::from(hi - lo) + 1];
    for (tick, event) in clip.plays_to_draw(from as u64, to.max(from) as u64, px_ticks) {
        let Some((len, key, velocity)) = note(event) else {
            continue;
        };
        let tick = tick as f64;
        let end = base + (((tick - base) / lp).floor() + 1.0) * lp;
        let x = m.floor_px(axis.x(tick));
        let right = (x + width(len)).min(m.floor_px(axis.x(end)));
        let reach = &mut drawn[usize::from(key - lo)];
        if right <= *reach {
            continue;
        }
        *reach = right;
        let y = m.floor_px(area.bottom() - t - (f32::from(key) - f32::from(lo) + 1.0) * p);
        painter.rect_filled(
            Rect::from_x_y_ranges(x..=right, y..=y + t),
            0.0,
            ink(velocity),
        );
    }
}

/// An automation clip's points where the painter's clip rect reaches, every loop pass, as a stepped area in `area` in
/// `ink`: from each point (every stored event: the control points and the steps between them) to the next, the last
/// one a pixel wide, a box from y = bottom − 1 px − value·height down to the bottom; before a pass's first point the
/// previous point (the pass before's last) holds from the pass start. A pass narrower than a pixel per
/// [`MAX_PLAYS_A_PIXEL`] points draws nothing, as for the notes.
fn automation(
    painter: &Painter,
    clip: &Clip,
    area: Rect,
    axis: TimeAxis,
    ink: Color32,
    m: &Metrics,
) {
    let pts: Vec<(f64, f32)> = (clip.events.iter())
        .filter_map(|e| match *e {
            Event::Automation { pos, value, .. } => Some((f64::from(pos), value)),
            _ => None,
        })
        .collect();
    let (Some(lp), Some(&(_, last))) = (clip.loop_ticks(), pts.last()) else {
        return;
    };
    let px = 1.0 / m.ppp();
    let px_ticks = f64::from(px / axis.beat_w) * f64::from(PPQ);
    if lp < px_ticks * (pts.len() as f64 / MAX_PLAYS_A_PIXEL).max(1.0) {
        return;
    }
    // Pass k's content starts at base + k·loop; pass 0 is the one the clip starts in (at its offset).
    let start = f64::from(clip.start_tick);
    let base = start - (clip.offset_beats * f64::from(PPQ)).rem_euclid(lp);
    let seen = painter.clip_rect();
    let from = axis.tick(seen.left()).max(start);
    let to = axis.tick(seen.right()).min(end_tick(clip));
    let x = |tick: f64| m.floor_px(axis.x(tick));
    let fill = |x0: f32, x1: f32, v: f32| {
        let top = area.bottom() - px - m.floor_px(v.clamp(0.0, 1.0) * area.height());
        if x1 > x0 {
            painter.rect_filled(
                Rect::from_x_y_ranges(x0..=x1, top..=area.bottom()),
                0.0,
                ink,
            );
        }
    };
    let passes =
        ((from - base) / lp).floor().max(0.0) as u64..((to - base) / lp).ceil().max(0.0) as u64;
    for k in passes {
        let at = base + k as f64 * lp;
        // The points from the last one at or before the view's left edge.
        let before = pts.partition_point(|p| at + p.0 <= from);
        if before == 0 {
            fill(x(at), x(at + pts[0].0), last);
        }
        for (j, &(pos, v)) in pts.iter().enumerate().skip(before.saturating_sub(1)) {
            let x0 = x(at + pos);
            if x0 > seen.right() {
                break;
            }
            fill(x0, pts.get(j + 1).map_or(x0 + px, |p| x(at + p.0)), v);
        }
    }
}

/// The light bubbles' label font (the floating buttons', `Undo` and `Redo`): the largest of 8, 7 and 6 pt at which
/// every one of `labels` fits [`LABEL_FIT`] of the diameter `d` (6 pt if none does).
pub(crate) fn label_font(ctx: &egui::Context, m: &Metrics, d: f32, labels: &[&str]) -> FontId {
    let width = |label: &str, font: &FontId| {
        ctx.fonts_mut(|f| {
            f.layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
    };
    [8.0, 7.0]
        .map(|pt| m.font_id_pt(pt))
        .into_iter()
        .find(|font| {
            labels
                .iter()
                .all(|label| width(label, font) <= LABEL_FIT * d)
        })
        .unwrap_or_else(|| m.font_id_pt(6.0))
}

/// The song tick where the clip ends.
pub fn end_tick(clip: &Clip) -> f64 {
    f64::from(clip.start_tick) + clip.visible_ticks()
}

/// Where the last clip of any lane ends, in beats.
pub fn song_end_beats(song: &Song) -> f64 {
    song.channels
        .iter()
        .flat_map(|ch| &ch.lanes)
        .flat_map(|lane| &lane.clips)
        .map(|clip| end_tick(clip) / f64::from(PPQ))
        .fold(0.0, f64::max)
}

/// The clip's block: from its start to the end of its visible length, across `row`'s height.
pub fn clip_rect(clip: &Clip, row: Rect, axis: TimeAxis) -> Rect {
    let start = f64::from(clip.start_tick);
    Rect::from_x_y_ranges(
        axis.x(start)..=axis.x(start + clip.visible_ticks()),
        row.y_range(),
    )
}

/// What a drag grabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Body,
    /// The left handle.
    Start,
    /// The right handle.
    End,
    /// The third handle, under an audio clip that does not loop: the end law, as a scale of the clip.
    Scale,
}

/// The clip fields a drag changes, with the visible length resolved to beats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: u32,
    pub visible: f64,
    pub offset: f64,
}

impl Span {
    pub fn of(clip: &Clip) -> Self {
        Self {
            start: clip.start_tick,
            visible: clip.visible_ticks() / f64::from(PPQ),
            offset: clip.offset_beats,
        }
    }

    /// Writes the span into `clip`. A visible length it does not change stays as stored (0 = the loop length).
    pub fn apply(self, clip: &mut Clip) {
        if self.visible != Self::of(clip).visible {
            clip.visible_len_beats = self.visible;
        }
        clip.start_tick = self.start;
        clip.offset_beats = self.offset;
    }
}

/// `offset` beats wrapped into the loop.
fn wrap(offset: f64, loop_beats: f64) -> f64 {
    if loop_beats > 0.0 {
        offset.rem_euclid(loop_beats)
    } else {
        offset
    }
}

/// `orig` after its `part` was dragged by `delta` ticks, on a `grid`-tick grid, for a loop `loop_beats` long, `min`
/// ticks the shortest a handle leaves it (1/(2d) beat):
/// - the body moves, its start snapped and never before the song start;
/// - the start handle trims or extends the clip with its content in place (the offset follows), its start
///   snapped, at most `min` before the end (on the tick at or before) and never before the song start;
/// - the end handle (and the scale handle) sets the visible length, its end snapped, at least `min` after the start
///   (on the tick at or after).
///
/// No movement changes nothing.
pub fn dragged(orig: Span, loop_beats: f64, part: Part, delta: f64, grid: f64, min: f64) -> Span {
    if delta == 0.0 {
        return orig;
    }
    let (g, ppq) = (grid, f64::from(PPQ));
    let snap = |tick: f64| ((tick / g).round() * g).round();
    let start = f64::from(orig.start);
    let end = start + orig.visible * ppq;
    match part {
        Part::Body => Span {
            start: snap(start + delta).max(0.0) as u32,
            ..orig
        },
        Part::End | Part::Scale => Span {
            visible: (snap(end + delta).max((start + min).ceil()) - start) / ppq,
            ..orig
        },
        Part::Start => {
            let new = snap(start + delta).min((end - min).floor()).max(0.0);
            let moved = (new - start) / ppq;
            Span {
                start: new as u32,
                visible: orig.visible - moved,
                offset: wrap(orig.offset + moved, loop_beats),
            }
        }
    }
}

/// The grid line nearest `tick` (≥ 0), as a whole tick: where the insert menu inserts.
fn cell(tick: f64, grid: f64) -> f64 {
    ((tick.max(0.0) / grid).round() * grid).round()
}

/// Adds a one-bar clip (`bar` beats) to `lane` from the grid line nearest `tick` (round(beat/unit)·unit);
/// returns its id.
pub fn add_clip(lane: &mut Lane, tick: f64, grid: f64, bar: u32) -> u32 {
    let start = (tick.max(0.0) / grid).round() * grid;
    lane.add_clip(start.round() as u32, f64::from(bar)).id
}

/// Copies clip `id` to right after itself, its length rounded up to the grid; returns the copy's id. Grid lines are
/// stored as whole ticks, so a start within half a tick of one counts as on it, and the end is rounded up from there
/// with half a tick of slack.
pub fn copy_clip(lane: &mut Lane, id: u32, grid: f64) -> Option<u32> {
    let mut copy = lane.clips.iter().find(|c| c.id == id)?.clone();
    let stored = f64::from(copy.start_tick);
    let line = (stored / grid).round() * grid;
    let start = if (stored - line).abs() <= 0.5 {
        line
    } else {
        stored
    };
    let end = stored + copy.visible_ticks();
    let after = ((end - 0.5 - start) / grid).ceil().max(1.0) * grid;
    // `as` saturates at the last tick.
    copy.start_tick = (start + after).round() as u32;
    lane.last_clip_id += 1;
    copy.id = lane.last_clip_id;
    lane.clips.push(copy);
    Some(lane.last_clip_id)
}

/// `paste` (as the automation editor's Paste): the earliest of `clips` lands on the grid line nearest `tick`,
/// the others keep their offsets from it. A lone clip goes on `track` when that is a track of its kind, else each goes
/// back on its own track; a track that is gone or of another kind takes nothing, nor does a hidden one (`Hide track`)
/// unless `show_hidden` (Settings `Show hidden tracks`). Returns the copies (new ids).
pub fn paste_clips(
    song: &mut Song,
    clips: &[(ClipRef, LaneKind, Clip)],
    tick: f64,
    (channel, lane): (usize, usize),
    grid: f64,
    show_hidden: bool,
) -> Vec<ClipRef> {
    let Some(first) = clips.iter().map(|c| c.2.start_tick).min() else {
        return Vec::new();
    };
    let at = (tick.max(0.0) / grid).round() * grid;
    let kind = |song: &Song, c: usize, l: usize| Some(song.channels.get(c)?.lanes.get(l)?.kind);
    let mut pasted = Vec::new();
    for (r, k, clip) in clips {
        let (c, l) = match clips.len() == 1 && kind(song, channel, lane) == Some(*k) {
            true => (channel, lane),
            false => (r.channel, r.lane),
        };
        let fits = |t: &&mut Lane| t.kind == *k && (show_hidden || !t.hidden);
        let Some(target) = lane_mut(song, c, l).filter(fits) else {
            continue;
        };
        let mut copy = clip.clone();
        // `as` saturates at the last tick.
        copy.start_tick = (at + f64::from(clip.start_tick - first)).round() as u32;
        target.last_clip_id += 1;
        copy.id = target.last_clip_id;
        pasted.push(ClipRef {
            channel: c,
            lane: l,
            id: copy.id,
        });
        target.clips.push(copy);
    }
    pasted
}

/// Removes clip `id`; returns whether it was there.
/// Copies `clips` (of any tracks) to right after themselves: one clip as [`copy_clip`] does, several by their span
/// together rounded up to the grid (the group keeps its shape); returns the copies.
pub fn copy_clips(song: &mut Song, clips: &[ClipRef], grid: f64) -> Vec<ClipRef> {
    if let [r] = clips {
        let lane = lane_mut(song, r.channel, r.lane);
        return (lane
            .and_then(|l| copy_clip(l, r.id, grid))
            .map(|id| ClipRef { id, ..*r }))
        .into_iter()
        .collect();
    }
    let spans: Vec<(f64, f64)> = (clips.iter().filter_map(|&r| find(song, r)))
        .map(|c| (f64::from(c.start_tick), end_tick(c)))
        .collect();
    let start = spans.iter().map(|s| s.0).fold(f64::MAX, f64::min);
    let end = spans.iter().map(|s| s.1).fold(f64::MIN, f64::max);
    let after = ((end - 0.5 - start) / grid).ceil().max(1.0) * grid;
    let mut copies = Vec::new();
    for &r in clips {
        let Some(lane) = lane_mut(song, r.channel, r.lane) else {
            continue;
        };
        let Some(mut copy) = lane.clips.iter().find(|c| c.id == r.id).cloned() else {
            continue;
        };
        // `as` saturates at the last tick.
        copy.start_tick = (f64::from(copy.start_tick) + after).round() as u32;
        lane.last_clip_id += 1;
        copy.id = lane.last_clip_id;
        copies.push(ClipRef { id: copy.id, ..r });
        lane.clips.push(copy);
    }
    copies
}

pub fn delete_clip(lane: &mut Lane, id: u32) -> bool {
    let count = lane.clips.len();
    lane.clips.retain(|c| c.id != id);
    lane.clips.len() != count
}

/// `Snap`: moves the clip's start to the nearest grid line.
pub fn quantize(clip: &mut Clip, grid: f64) {
    clip.start_tick = ((f64::from(clip.start_tick) / grid).round() * grid).round() as u32;
}

/// Song tick `tick` is strictly inside the clip.
fn contains(clip: &Clip, tick: u64) -> bool {
    let tick = tick as f64;
    f64::from(clip.start_tick) < tick && tick < end_tick(clip)
}

/// The clip's content repeats: it shows more than one pass of its loop.
pub fn loops(clip: &Clip) -> bool {
    clip.loop_ticks()
        .is_some_and(|lp| clip.visible_ticks() > lp)
}

/// `Slice`: splits clip `id` at song tick `at`, strictly inside it. The clip ends there and a new clip, whose id
/// is returned, plays the rest, so every note stays where it played.
pub fn slice(lane: &mut Lane, id: u32, at: u64) -> Option<u32> {
    let index = lane.clips.iter().position(|c| c.id == id)?;
    let clip = &mut lane.clips[index];
    if !contains(clip, at) {
        return None;
    }
    let (end, ppq) = (end_tick(clip), f64::from(PPQ));
    let head = (at as f64 - f64::from(clip.start_tick)) / ppq;
    let mut tail = clip.clone();
    clip.visible_len_beats = head;
    tail.start_tick = at as u32;
    tail.visible_len_beats = (end - at as f64) / ppq;
    tail.offset_beats = wrap(tail.offset_beats + head, tail.loop_len_beats);
    lane.last_clip_id += 1;
    tail.id = lane.last_clip_id;
    lane.clips.push(tail);
    Some(lane.last_clip_id)
}

/// The most plays `Unlink loop` writes out: what the reader's base budget holds, the memory any file may take
/// ([`crate::flm::BUDGET_BASE`]).
const MAX_UNLINKED: usize = crate::flm::BUDGET_BASE / std::mem::size_of::<Event>();

/// `Unlink loop` applies: the clip [`loops`], and every pass written out fits what a file holds: content positions
/// within u32 ticks (`EVN2` @0), and at most [`MAX_UNLINKED`] plays (a bound from the passes, so a crafted visible
/// length is refused without walking them).
pub fn can_unlink(clip: &Clip) -> bool {
    let visible = clip.visible_ticks();
    clip.loop_ticks().is_some_and(|lp| {
        let passes = (visible / lp).ceil() + 1.0;
        loops(clip)
            && visible <= f64::from(u32::MAX)
            && passes * clip.events.len() as f64 <= MAX_UNLINKED as f64
    })
}

/// `Unlink loop`: writes every pass of the loop out as content of its own, one pass as long as the clip. `false`, and
/// nothing changes, where it does not apply ([`can_unlink`]).
pub fn unlink_loop(clip: &mut Clip) -> bool {
    if !can_unlink(clip) {
        return false;
    }
    let start = u64::from(clip.start_tick);
    let plays = || clip.events_in_song_range(start, end_tick(clip).ceil() as u64);
    let events = plays()
        .map(|(tick, e)| at_pos(*e, (tick - start) as u32))
        .collect();
    // An automation point's segment shape goes with each of its plays.
    let mut shapes: Vec<_> = (plays())
        .filter_map(|(tick, e)| match *e {
            Event::Automation { pos, control, .. } if control => Some((
                (tick - start) as u32,
                crate::song::points::shape_at(clip, pos),
            )),
            _ => None,
        })
        .filter(|s| s.1 != Default::default())
        .collect();
    shapes.dedup_by_key(|s| s.0);
    let visible = clip.visible_ticks() / f64::from(PPQ);
    (clip.events, clip.file.shapes) = (events, shapes);
    (
        clip.loop_len_beats,
        clip.visible_len_beats,
        clip.offset_beats,
    ) = (visible, visible, 0.0);
    true
}

/// `event` moved to content position `at`.
pub fn at_pos(mut event: Event, at: u32) -> Event {
    match &mut event {
        Event::Note { pos, .. }
        | Event::Drum { pos, .. }
        | Event::Automation { pos, .. }
        | Event::Envelope { pos, .. } => *pos = at,
    }
    event
}

/// The `More...` dropdown's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoreOp {
    Slice,
    UnlinkLoop,
    Combine,
    Mute,
    Unmute,
}

/// The `More...` dropdown for the selected clip, one column, with the playhead at song tick `playhead`.
pub fn more_items(clips: &[&Clip], playhead: u64) -> Vec<PopupItem<MoreOp>> {
    let any = |f: &dyn Fn(&Clip) -> bool| clips.iter().any(|c| f(c));
    let one = (clips.len() == 1).then(|| clips[0]);
    [
        (MoreOp::Slice, "Slice", any(&|c| contains(c, playhead))),
        (
            MoreOp::UnlinkLoop,
            "Unlink loop",
            one.is_some_and(can_unlink),
        ),
        // Not built: it needs a track with two selected note clips merged.
        (MoreOp::Combine, "Combine", false),
        (MoreOp::Mute, "Mute", any(&|c| !c.muted)),
        (MoreOp::Unmute, "Unmute", any(&|c| c.muted)),
    ]
    .into_iter()
    .enumerate()
    .map(|(row, (op, label, enabled))| PopupItem {
        disabled: !enabled,
        ..PopupItem::new(op, 0, row, label)
    })
    .collect()
}

/// What a floating row is placed in: the editor's visible grid (the viewport) and how its content
/// maps to the screen (an origin's x and y, then points per unit along x and y). Any change of either is a scroll, a
/// zoom or a new grid rect, which hides the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GridView {
    pub grid: Rect,
    pub map: [f32; 4],
}

/// The selection a floating row floats over: its bounds, whether it is one row (track, key or pad) high, and the
/// handles drawn on it, which the row keeps clear of.
pub(crate) struct Selection<'a> {
    pub rect: Rect,
    pub one_row: bool,
    pub handles: &'a [Rect],
}

/// A floating row's state, shared by the playlist, the piano roll and the drum editor. Any scroll or zoom of the editor
/// (or a change of its grid rect) hides the row at once, whether or not the selection stays visible; it is not dragged
/// along. It comes back only when a later gesture re-shows it: a tap or release on the selection, or Select All, which
/// anchors at the viewport's centre (these editors have no focus cell). A press that starts on the row itself or off
/// the editor (a popup or an overlay over it) re-shows nothing, so the row's own buttons keep it where it is; a gesture
/// that changes the selection (a box, a long press) re-shows it at its release even when the release lies off the new
/// selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FloatingRow {
    /// Shown, anchored at this finger position; `None`: hidden.
    anchor: Option<Pos2>,
    /// The view of the last frame that had a selection (none without one, so a selection's first view is no change).
    view: Option<GridView>,
    /// A press that may re-show the row is down: the selection's bounds at the press.
    press: Option<Option<Rect>>,
    /// The row as drawn last frame.
    drawn: Option<Rect>,
    /// The pointer is over an overlay drawn in the editor's layer (the rack): a press there is not the editor's. The
    /// host sets it each frame.
    pub(crate) covered: bool,
    /// When the row last showed from hidden (input time): its pop-in.
    shown_at: f64,
}

impl FloatingRow {
    /// `Select All`: shows the row anchored at the viewport's centre (these editors have no focus cell).
    pub(crate) fn select_all(&mut self) {
        self.anchor = self.view.map(|v| v.grid.center());
    }

    /// Hides the row until a later gesture re-shows it (the keymap's editing actions, `ui/edit_keys.rs`).
    pub(crate) fn hide(&mut self) {
        self.anchor = None;
    }

    /// The row as drawn last frame.
    #[cfg(test)]
    pub(crate) fn rect(&self) -> Option<Rect> {
        self.drawn
    }

    /// The row showed in its last frame.
    pub(crate) fn showing(&self) -> bool {
        self.drawn.is_some()
    }

    /// One frame: `sel` is the selection's bounds, `press` and `release` where a pointer button went down on the
    /// editor and up this frame.
    fn track(
        &mut self,
        view: GridView,
        sel: Option<Rect>,
        press: Option<Pos2>,
        release: Option<Pos2>,
    ) {
        if sel.is_none() {
            (self.anchor, self.view) = (None, None);
        } else if self.view.replace(view).is_some_and(|v| v != view) {
            // A scroll or zoom also ends the gesture's claim on the row: its release shows nothing.
            (self.anchor, self.press) = (None, None);
        }
        if press.is_some_and(|p| !self.drawn.is_some_and(|row| row.contains(p))) {
            self.press = Some(sel);
            // Touch-down hides the row; the release may bring it back.
            self.anchor = None;
        }
        if let Some(p) = release
            && let Some(before) = self.press.take()
            && let Some(sel) = sel
            && (sel.contains(p) || before != Some(sel))
        {
            self.anchor = Some(p);
        }
    }

    /// The row of light bubbles ([`bubble_button`]), one per label, over `sel` in `view` (placed by
    /// [`place_row`]) while shown; button `i` has the id `(id, i)`. Returns the index and centre of the one clicked.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        sel: Option<Selection>,
        view: GridView,
        labels: &[&str],
        cx: &UiCtx,
    ) -> Option<(usize, Pos2)> {
        let (press, release) = ui.input(|i| {
            let at = |down: bool| {
                i.events.iter().find_map(|e| match *e {
                    egui::Event::PointerButton { pos, pressed, .. } if pressed == down => Some(pos),
                    _ => None,
                })
            };
            (at(true), at(false))
        });
        // On the grid, and not under a popup or an overlay.
        let press = press.filter(|&p| {
            !self.covered && view.grid.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
        });
        self.track(view, sel.as_ref().map(|s| s.rect), press, release);
        let (c, m) = (&cx.theme.colors, cx.m);
        let size = vec2(m.u(ROW_SLOT) * labels.len() as f32, m.u(ROW_B));
        let px = 1.0 / m.ppp();
        let was = self.drawn.is_some();
        self.drawn = (self.anchor.zip(sel))
            .and_then(|(finger, sel)| place_row(&sel, finger, view.grid, size, m.u(ROW_M), px));
        let row = self.drawn?;
        // The pop-in: shown from hidden, every circle's radius is scaled by 1.0515·sin(1.885·(1 − a)), a ← 0.6·a per
        // frame (per 60 Hz frame of input time, as the editors' pops), 0 below 0.01. Input never waits for it.
        let now = ui.input(|i| i.time);
        if !was {
            self.shown_at = now;
        }
        let a = pop_in(ROW_POP, now - self.shown_at);
        if a > 0.0 {
            ui.ctx().request_repaint();
        }
        let (d, scale) = (m.u(BUTTON_D), row_pop_scale(a));
        let font = label_font(ui.ctx(), m, d, labels);
        let mut clicked = None;
        for (i, &label) in labels.iter().enumerate() {
            let center = pos2(
                row.left() + (i as f32 + 0.5) * m.u(ROW_SLOT),
                row.center().y,
            );
            // Light floating-row bubbles, not `background` discs in a `text` ring. The button takes its full disc
            // while it pops in.
            let button = Id::new((id, i));
            if bubble_button(ui, button, center, d, scale, label, font.clone(), c).clicked() {
                clicked = Some((i, center));
            }
        }
        clicked
    }
}

/// A floating row's pop-in scale at `a` (1 when settled).
pub(crate) fn row_pop_scale(a: f32) -> f32 {
    if a <= 0.0 {
        1.0
    } else {
        1.0515 * (1.885 * (1.0 - a)).sin()
    }
}

/// Where a floating row of `size` (W × B) goes over `sel` with the finger at `finger`, in the viewport `view`, with
/// the margin `m`; `px` (a device pixel) is the sideways slide's step. `None`: hidden.
/// - Vertically, above the selection (top = sel.top − B − m) or below it (top = sel.bottom + m). Where both have room
///   (B + 2m), above for a selection one row/track high, else the side nearer the finger; where only one has room,
///   that one; where neither has, inside the selection, inset m from its top or bottom edge (picked on the same terms
///   as above), clamped to the viewport.
/// - Horizontally, centred on the finger x, then slid right 1 px at a time while it fits, then left, until it is fully
///   inside the viewport and overlaps no visible handle. With no such place, the centred row moved into the viewport.
/// - A selection off-screen sideways holds the row at the viewport edge (x = left edge, or right edge − W) at the
///   computed height; one off-screen vertically (the needed position not fully inside the viewport) hides it, as does
///   a viewport narrower than W + 2m.
pub(crate) fn place_row(
    sel: &Selection,
    finger: Pos2,
    view: Rect,
    size: Vec2,
    m: f32,
    px: f32,
) -> Option<Rect> {
    let (w, b) = (size.x, size.y);
    if view.width() < w + 2.0 * m {
        return None;
    }
    let s = sel.rect;
    let near_top = sel.one_row || finger.y <= s.center().y;
    let room = b + 2.0 * m;
    let (above, below) = (s.top() - b - m, s.bottom() + m);
    let top = match (
        s.top() - view.top() >= room,
        view.bottom() - s.bottom() >= room,
    ) {
        (true, true) if near_top => above,
        (true, true) => below,
        (true, false) => above,
        (false, true) => below,
        (false, false) => {
            let inside = if near_top {
                s.top() + m
            } else {
                s.bottom() - m - b
            };
            inside.min(view.bottom() - b).max(view.top())
        }
    };
    if top < view.top() || top + b > view.bottom() {
        return None;
    }
    let left = if s.right() <= view.left() {
        view.left()
    } else if s.left() >= view.right() {
        view.right() - w
    } else {
        slide(finger.x - w / 2.0, top, size, view, sel.handles, px)
    };
    Some(Rect::from_min_size(pos2(left, top), size))
}

/// The row's left edge from `x0`: `px` at a time to the right while its right end stays in `view`, then to the left
/// while its left end does, the first place fully in `view` that overlaps no visible handle (places still partly
/// outside on the near side are skipped at once); else `x0` moved into `view`.
fn slide(x0: f32, top: f32, size: Vec2, view: Rect, handles: &[Rect], px: f32) -> f32 {
    let (lo, hi) = (view.left(), view.right() - size.x);
    // Only the visible handles across the row's height can block it: their x spans, found once (a dense selection has
    // a handle per note).
    let blocks: Vec<(f32, f32)> = (handles.iter())
        .map(|h| h.intersect(view))
        .filter(|h| h.is_positive() && h.top() < top + size.y && top < h.bottom())
        .map(|h| (h.left(), h.right()))
        .collect();
    let free =
        |&x: &f32| (lo..=hi).contains(&x) && !blocks.iter().any(|&(l, r)| l < x + size.x && x < r);
    // The first step whose place is no longer outside, going `dir` from `x0`.
    let steps = |outside: f32, dir: f32| {
        let first = (outside / px).ceil().max(0.0) as u32;
        (first..).map(move |k| x0 + dir * k as f32 * px)
    };
    (steps(lo - x0, 1.0).take_while(|&x| x <= hi).find(free))
        .or_else(|| steps(x0 - hi, -1.0).take_while(|&x| x >= lo).find(free))
        .unwrap_or_else(|| x0.min(hi).max(lo))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use honeydew_audio::sample::SampleData;

    use super::*;
    use eframe::egui::{Rangef, vec2};

    const BEAT: f64 = PPQ as f64;

    /// A notes clip at `start` ticks, `visible` beats long (0 = its loop), looping every `looped` beats and
    /// starting `offset` beats into its content.
    fn clip(start: u32, visible: f64, looped: f64, offset: f64) -> Clip {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(start, looped);
        clip.visible_len_beats = visible;
        clip.offset_beats = offset;
        clip.clone()
    }

    fn note(pos: u32, key: u8) -> Event {
        Event::Note {
            pos,
            len_beats: 0.5,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }

    fn key(event: &Event) -> u8 {
        match *event {
            Event::Note { key, .. } => key,
            _ => unreachable!(),
        }
    }

    /// Every play of the lane's clips as (song tick, key), in time order.
    fn plays(clips: &[Clip]) -> Vec<(u64, u8)> {
        let mut plays: Vec<_> = clips
            .iter()
            .flat_map(|c| c.events_in_song_range(0, 100_000).map(|(t, e)| (t, key(e))))
            .collect();
        plays.sort();
        plays
    }

    fn span(start: u32, visible: f64, offset: f64) -> Span {
        Span {
            start,
            visible,
            offset,
        }
    }

    #[test]
    fn the_track_lists_scroll_keys_a_single_visible_row_too() {
        // A track area 32 points tall (rows 64 apart) shows one row except across a row boundary; each point of
        // scroll must still move `list_top` by a point, or a scroll to the next row would not hide the row.
        let mut song = Song::default();
        for _ in 0..4 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let tracks = track_rows(&song, false);
        let (top, view_h, pitch) = (100.0_f32, 32.0_f32, 64.0_f32);
        let list_top_at = |scroll: f32| {
            let first = (scroll / pitch).floor() as usize;
            let end = (((scroll + view_h) / pitch).ceil() as usize).min(tracks.len());
            // The lane rects leave the 1-px gap under each row clear.
            let rows: Vec<_> = (first..end)
                .map(|i| {
                    let y = top - scroll + i as f32 * pitch;
                    let rect = Rect::from_x_y_ranges(0.0..=100.0, y..=y + pitch - 1.0);
                    (tracks[i].channel, tracks[i].lane, rect)
                })
                .collect();
            list_top(&song, &rows, 1.0, false)
        };
        // At 63 row 0 is first and row 1 is below it; at 64 row 1 is alone. The list's top still moves by the point.
        assert_ne!(list_top_at(63.0), list_top_at(64.0));
        for scroll in 0..=200 {
            let scroll = scroll as f32;
            assert_eq!(list_top_at(scroll), top - scroll, "scroll {scroll}");
        }
    }

    #[test]
    fn the_axis_maps_ticks_to_x_and_back() {
        let axis = TimeAxis {
            origin: 48.0,
            beat_w: 40.0,
        };
        assert_eq!(axis.x(0.0), 48.0);
        assert_eq!(axis.x(64.0), 68.0);
        // One 4/4 bar.
        assert_eq!(axis.x(512.0), 208.0);
        assert_eq!(axis.tick(208.0), 512.0);
        // Scrolled by two beats: tick 256 sits on the grid's left edge.
        let scrolled = TimeAxis {
            origin: 48.0 - 80.0,
            beat_w: 40.0,
        };
        assert_eq!(scrolled.x(256.0), 48.0);
    }

    /// A zoom range for the tests: 1/8 to 4 times the default beat.
    const RANGE: [f32; 2] = [-3.0, 2.0];

    #[test]
    fn zooming_keeps_the_tick_under_the_pointer() {
        // A default beat of 38.4 points (1.2 U at U 32), scrolled 2 beats, the pointer 200 points into the grid.
        let (scroll, zoom) = zoomed(2.0, 0.0, 2.0, 200.0, 38.4, RANGE);
        assert_eq!(zoom, 1.0);
        let before = 2.0 + 200.0 / 38.4;
        assert!((scroll + 200.0 / 76.8 - before).abs() < 1e-4, "{scroll}");
        // Clamped to 1/8 .. 4 times the default beat, and never scrolled before the song start.
        assert_eq!(zoomed(0.0, 1.5, 4.0, 0.0, 38.4, RANGE), (0.0, 2.0));
        assert_eq!(zoomed(0.0, -2.5, 0.25, 0.0, 38.4, RANGE), (0.0, -3.0));
        assert_eq!(zoomed(0.0, 0.0, 0.5, 300.0, 38.4, RANGE).0, 0.0);
    }

    #[test]
    fn a_clip_spans_its_row_from_its_start_to_the_end_of_its_visible_length() {
        let axis = TimeAxis {
            origin: 48.0,
            beat_w: 40.0,
        };
        let row = Rect::from_x_y_ranges(48.0..=1170.0, 112.0..=175.0);
        // One bar from beat 2.
        assert_eq!(
            clip_rect(&clip(256, 0.0, 4.0, 0.0), row, axis),
            Rect::from_x_y_ranges(128.0..=288.0, 112.0..=175.0)
        );
        // Six beats of a one-bar loop.
        assert_eq!(
            clip_rect(&clip(0, 6.0, 4.0, 0.0), row, axis).x_range(),
            Rangef::new(48.0, 288.0)
        );
    }

    #[test]
    fn moving_snaps_the_start_keeps_the_length_and_stops_at_the_song_start() {
        let bar = span(512, 4.0, 0.0);
        assert_eq!(
            dragged(bar, 4.0, Part::Body, 1.4 * BEAT, 128.0, 16.0),
            span(640, 4.0, 0.0)
        );
        assert_eq!(
            dragged(bar, 4.0, Part::Body, 1.6 * BEAT, 128.0, 16.0),
            span(768, 4.0, 0.0)
        );
        // On the bar grid.
        assert_eq!(dragged(bar, 4.0, Part::Body, 1.6 * BEAT, 512.0, 16.0), bar);
        assert_eq!(
            dragged(bar, 4.0, Part::Body, 2.1 * BEAT, 512.0, 16.0),
            span(1024, 4.0, 0.0)
        );
        assert_eq!(
            dragged(bar, 4.0, Part::Body, -9.0 * BEAT, 128.0, 16.0),
            span(0, 4.0, 0.0)
        );
        // An off-grid clip that has not moved stays where it is.
        let off = span(100, 4.0, 0.0);
        assert_eq!(dragged(off, 4.0, Part::Body, 0.0, 128.0, 16.0), off);
    }

    #[test]
    fn the_end_handle_snaps_the_end_and_keeps_half_a_step() {
        // Snapped, at least start + 1/(2d) beat (d = 4: 16 ticks), on or off the grid.
        let bar = span(512, 4.0, 0.0);
        // Past the loop end: the content repeats.
        assert_eq!(
            dragged(bar, 4.0, Part::End, 2.2 * BEAT, 128.0, 16.0),
            span(512, 6.0, 0.0)
        );
        assert_eq!(
            dragged(bar, 4.0, Part::End, -9.0 * BEAT, 128.0, 16.0),
            span(512, 0.125, 0.0)
        );
        // The third handle's end law is the same.
        assert_eq!(
            dragged(bar, 4.0, Part::Scale, -9.0 * BEAT, 128.0, 16.0),
            span(512, 0.125, 0.0)
        );
        // The step grid.
        assert_eq!(
            dragged(bar, 4.0, Part::End, -0.3 * BEAT, 32.0, 16.0),
            span(512, 3.75, 0.0)
        );
    }

    #[test]
    fn the_start_handle_trims_or_extends_with_the_content_in_place() {
        let bar = span(512, 4.0, 0.0);
        // Trimming a beat: the window starts a beat into the content.
        assert_eq!(
            dragged(bar, 4.0, Part::Start, BEAT, 128.0, 16.0),
            span(640, 3.0, 1.0)
        );
        // Extending a beat: the window starts on the loop's last beat.
        assert_eq!(
            dragged(bar, 4.0, Part::Start, -BEAT, 128.0, 16.0),
            span(384, 5.0, 3.0)
        );
        // At most end − 1/(2d) beat (16 ticks before the end), and never before the song start.
        assert_eq!(
            dragged(bar, 4.0, Part::Start, 9.0 * BEAT, 128.0, 16.0),
            span(1008, 0.125, 3.875)
        );
        assert_eq!(
            dragged(bar, 4.0, Part::Start, -9.0 * BEAT, 128.0, 16.0),
            span(0, 8.0, 0.0)
        );

        // The notes left in the window play where they played before.
        let mut c = clip(512, 0.0, 4.0, 0.0);
        c.events = vec![note(0, 60), note(128, 62), note(256, 64)];
        let before = plays(std::slice::from_ref(&c));
        dragged(Span::of(&c), 4.0, Part::Start, BEAT, 128.0, 16.0).apply(&mut c);
        assert_eq!(plays(&[c]), before[1..]);
    }

    #[test]
    fn the_start_handle_keeps_every_play_of_an_offset_loop_where_it_was() {
        // Six beats of a four-beat loop that starts a beat into its content.
        let mut c = clip(512, 6.0, 4.0, 1.0);
        c.events = vec![note(0, 60), note(128, 62), note(256, 64), note(384, 65)];
        let before = plays(std::slice::from_ref(&c));
        for beats in [1.0, 2.5, -1.0, -3.0] {
            let mut d = c.clone();
            dragged(Span::of(&c), 4.0, Part::Start, beats * BEAT, 128.0, 16.0).apply(&mut d);
            let after = plays(std::slice::from_ref(&d));
            let start = u64::from(d.start_tick);
            assert_eq!(end_tick(&d), end_tick(&c), "{beats}");
            if beats > 0.0 {
                // Trimmed: what is left plays as before.
                let kept: Vec<_> = before.iter().filter(|p| p.0 >= start).copied().collect();
                assert_eq!(after, kept, "{beats}");
            } else {
                // Extended: the old window plays as before, and the new part plays the loop's earlier passes.
                let old: Vec<_> = after.iter().filter(|p| p.0 >= 512).copied().collect();
                assert_eq!(old, before, "{beats}");
                for &(tick, key) in after.iter().filter(|p| p.0 < 512) {
                    assert!(
                        before.contains(&(tick + 512, key)) || tick + 512 >= 1280,
                        "{beats}: {tick}"
                    );
                }
            }
        }
    }

    #[test]
    fn copy_puts_the_clip_right_after_itself_rounded_up_to_the_grid_with_a_new_id() {
        let mut lane = Lane::new(LaneKind::Notes);
        let muted = lane.add_clip(512, 4.0);
        muted.events = vec![note(0, 60)];
        muted.muted = true;
        lane.add_clip(0, 3.5);
        assert_eq!(copy_clip(&mut lane, 1, 128.0), Some(3));
        let copy = &lane.clips[2];
        assert_eq!(
            (copy.id, copy.start_tick, copy.events.clone(), copy.muted),
            (3, 1024, vec![note(0, 60)], true)
        );
        // 3.5 beats, rounded up to a bar.
        assert_eq!(copy_clip(&mut lane, 2, 512.0), Some(4));
        assert_eq!(
            (lane.clips[3].start_tick, lane.clips[3].muted),
            (512, false)
        );
        assert_eq!(lane.last_clip_id, 4);
        assert_eq!(copy_clip(&mut lane, 9, 128.0), None);
    }

    #[test]
    fn at_subdivision_3_a_copy_lands_on_the_clips_end_step() {
        // Steps of 128/3 ticks, stored rounded: 0, 43, 85, 128, 171, 213 …
        let g = 128.0 / 3.0;
        for (start, end, want) in [
            (0, 43, 43),
            (0, 171, 171),
            (43, 85, 85),
            (43, 213, 213),
            (85, 171, 171),
        ] {
            let mut lane = Lane::new(LaneKind::Notes);
            lane.add_clip(start, 4.0).visible_len_beats = f64::from(end - start) / 128.0;
            copy_clip(&mut lane, 1, g);
            assert_eq!(lane.clips[1].start_tick, want, "{start}..{end}");
        }
    }

    #[test]
    fn every_on_grid_clip_copies_onto_its_end_at_fractional_steps() {
        for d in [3u32, 5, 6, 7] {
            let g = 128.0 / f64::from(d);
            let line = |i: u32| (f64::from(i) * g).round() as u32;
            for i in 0..2 * d {
                for n in 1..=2 * d {
                    let (start, end) = (line(i), line(i + n));
                    let mut lane = Lane::new(LaneKind::Notes);
                    lane.add_clip(start, 4.0).visible_len_beats = f64::from(end - start) / 128.0;
                    copy_clip(&mut lane, 1, g);
                    assert_eq!(lane.clips[1].start_tick, end, "d {d}: {start}..{end}");
                }
            }
        }
    }

    #[test]
    fn a_shrunk_end_is_a_whole_tick() {
        // At subdivision 3 the end handle stops half a step (21.33 ticks) after the start: tick 22, the tick at or
        // after; the start handle at the tick at or before the end's less that, 106 for an end at 128.
        let s = dragged(
            span(0, 4.0, 0.0),
            4.0,
            Part::End,
            -600.0,
            128.0 / 3.0,
            128.0 / 6.0,
        );
        assert_eq!(s.visible * 128.0, 22.0);
        let s = dragged(
            span(0, 1.0, 0.0),
            4.0,
            Part::Start,
            600.0,
            128.0 / 3.0,
            128.0 / 6.0,
        );
        assert_eq!(s.start, 106);
    }

    #[test]
    fn a_copy_past_the_last_tick_stops_on_it() {
        let mut lane = Lane::new(LaneKind::Notes);
        lane.add_clip(u32::MAX - 100, 4.0);
        assert_eq!(copy_clip(&mut lane, 1, 128.0), Some(2));
        assert_eq!(lane.clips[1].start_tick, u32::MAX);
    }

    #[test]
    fn a_group_copy_keeps_its_shape_on_new_ids_and_stops_on_the_last_tick() {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let at = |c: usize, l: usize, id: u32| ClipRef {
            channel: c,
            lane: l,
            id,
        };
        // Beats 0–1 and 2–3 on two tracks: the span, three beats, is the step.
        song.channels[1].lanes[0].add_clip(0, 1.0);
        song.channels[2].lanes[0].add_clip(256, 1.0);
        let copies = copy_clips(&mut song, &[at(1, 0, 1), at(2, 0, 1)], 128.0);
        assert_eq!(copies, [at(1, 0, 2), at(2, 0, 2)]);
        let start = |c: usize| song.channels[c].lanes[0].clips[1].start_tick;
        assert_eq!((start(1), start(2)), (384, 640));
        // Near the end of time the copies stop on the last tick instead of wrapping.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        let lane = &mut song.channels[1].lanes[0];
        lane.add_clip(u32::MAX - 600, 1.0);
        lane.add_clip(u32::MAX - 100, 1.0);
        let copies = copy_clips(&mut song, &[at(1, 0, 1), at(1, 0, 2)], 128.0);
        assert_eq!(copies, [at(1, 0, 3), at(1, 0, 4)]);
        let starts: Vec<u32> = (song.channels[1].lanes[0].clips.iter())
            .map(|c| c.start_tick)
            .collect();
        assert_eq!(starts, [u32::MAX - 600, u32::MAX - 100, u32::MAX, u32::MAX]);
    }

    #[test]
    fn delete_removes_that_clip_only() {
        let mut lane = Lane::new(LaneKind::Drums);
        lane.add_clip(0, 4.0);
        lane.add_clip(512, 4.0);
        assert!(delete_clip(&mut lane, 1));
        assert_eq!(lane.clips.iter().map(|c| c.id).collect::<Vec<_>>(), [2]);
        assert!(!delete_clip(&mut lane, 1));
        // Ids are never reused.
        assert_eq!(lane.add_clip(0, 4.0).id, 3);
    }

    #[test]
    fn a_new_clip_is_one_bar_from_the_grid_line_at_or_before_the_tap() {
        let mut lane = Lane::new(LaneKind::Notes);
        assert_eq!(add_clip(&mut lane, 700.0, 128.0, 4), 1);
        assert_eq!(add_clip(&mut lane, 700.0, 512.0, 4), 2);
        assert_eq!(add_clip(&mut lane, -5.0, 128.0, 4), 3);
        let made: Vec<_> = lane
            .clips
            .iter()
            .map(|c| (c.start_tick, c.visible_len_beats, c.loop_len_beats))
            .collect();
        assert_eq!(made, [(640, 0.0, 4.0), (512, 0.0, 4.0), (0, 0.0, 4.0)]);
    }

    #[test]
    fn snap_moves_the_start_to_the_nearest_grid_line() {
        let mut c = clip(200, 0.0, 4.0, 0.0);
        quantize(&mut c, 128.0);
        assert_eq!(c.start_tick, 256);
        let mut c = clip(700, 0.0, 4.0, 0.0);
        quantize(&mut c, 512.0);
        assert_eq!(c.start_tick, 512);
    }

    #[test]
    fn slice_splits_inside_the_clip_and_the_halves_play_what_it_played() {
        let mut lane = Lane::new(LaneKind::Notes);
        let c = lane.add_clip(512, 2.0);
        // Three passes of a two-beat loop.
        c.visible_len_beats = 6.0;
        c.events = vec![note(0, 60), note(128, 62)];
        c.muted = true;
        let before = plays(&lane.clips);
        assert_eq!(slice(&mut lane, 1, 512), None);
        assert_eq!(slice(&mut lane, 1, 512 + 768), None);
        assert_eq!(slice(&mut lane, 1, 512 + 320), Some(2));
        assert_eq!(plays(&lane.clips), before);
        let (a, b) = (&lane.clips[0], &lane.clips[1]);
        assert_eq!((a.start_tick, a.visible_len_beats), (512, 2.5));
        assert_eq!(
            (b.start_tick, b.visible_len_beats, b.offset_beats),
            (832, 3.5, 0.5)
        );
        assert!(a.muted && b.muted);
    }

    #[test]
    fn unlink_loop_writes_every_pass_out_once() {
        let mut c = clip(512, 5.0, 2.0, 1.0);
        c.events = vec![note(0, 60), note(128, 62)];
        assert!(loops(&c));
        let before = plays(std::slice::from_ref(&c));
        unlink_loop(&mut c);
        assert!(!loops(&c));
        assert_eq!(
            (c.loop_len_beats, c.visible_len_beats, c.offset_beats),
            (5.0, 5.0, 0.0)
        );
        assert_eq!(
            c.events.iter().map(Event::pos).collect::<Vec<_>>(),
            [0, 128, 256, 384, 512]
        );
        assert_eq!(plays(&[c]), before);
    }

    #[test]
    fn unlink_loop_refuses_more_than_a_file_can_hold() {
        let unlink_on = |c: &Clip| !more_items(&[c], 0)[1].disabled;
        // A crafted visible length: a billion beats of a one-bar loop, past the u32 ticks a content position has.
        let mut hostile = clip(512, 1e9, 4.0, 0.0);
        hostile.events = vec![note(0, 60)];
        // Inside the u32 ticks, but 30 million passes of 100 notes.
        let mut crowded = clip(0, 3e7, 1.0, 0.0);
        crowded.events = (0..100).map(|i| note(i, 60)).collect();
        // Four passes of one note, well inside the budget, but its plays from the fourth pass on lie past u32::MAX
        // ticks: only the u32 guard refuses it.
        let mut past_u32 = clip(0, 4e7, 1e7, 0.0);
        past_u32.events = vec![note(1_000_000_000, 60)];
        for c in [&mut hostile, &mut crowded, &mut past_u32] {
            assert!(loops(c) && !unlink_on(c));
            let before = c.clone();
            assert!(!unlink_loop(c));
            assert_eq!(*c, before);
        }
        // A long but sane one still unlinks.
        let mut long = clip(0, 4000.0, 4.0, 0.0);
        long.events = vec![note(0, 60)];
        assert!(unlink_on(&long) && unlink_loop(&mut long));
        assert_eq!(long.events.len(), 1000);
    }

    #[test]
    fn more_enables_slice_inside_unlink_on_a_loop_and_mute_or_unmute_by_the_flag() {
        let enabled = |c: &Clip, playhead| {
            more_items(&[c], playhead)
                .iter()
                .map(|i| (i.label.to_string(), !i.disabled))
                .collect::<Vec<_>>()
        };
        let pairs = |v: [(&str, bool); 5]| v.map(|(l, e)| (l.to_string(), e)).to_vec();
        let mut looping = clip(512, 6.0, 4.0, 0.0);
        assert_eq!(
            enabled(&looping, 600),
            pairs([
                ("Slice", true),
                ("Unlink loop", true),
                ("Combine", false),
                ("Mute", true),
                ("Unmute", false),
            ])
        );
        looping.muted = true;
        assert_eq!(
            enabled(&looping, 600)[3..],
            pairs([
                ("", false),
                ("", false),
                ("", false),
                ("Mute", false),
                ("Unmute", true),
            ])[3..]
        );
        // The playhead on the clip's start is not inside it, and a single pass does not loop.
        let one_pass = enabled(&clip(512, 0.0, 4.0, 0.0), 512);
        let on: Vec<_> = one_pass.iter().map(|(_, on)| *on).collect();
        assert_eq!(on, [false, false, false, true, false]);
    }

    // ---- the floating row: showing, hiding and placement ----

    /// A viewport 1000 × 500, a row 100 × 40 (W × B), m = 5: a side has room with 50 between the selection and the
    /// viewport's edge.
    const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 500.0));
    const ROW: Vec2 = vec2(100.0, 40.0);

    /// The row's top-left for a selection over x 400..500 and `y`, the finger at `finger`, with `handles`.
    fn placed(y: (f32, f32), one_row: bool, finger: Pos2, handles: &[Rect]) -> Option<Pos2> {
        let rect = Rect::from_x_y_ranges(400.0..=500.0, y.0..=y.1);
        let sel = Selection {
            rect,
            one_row,
            handles,
        };
        place_row(&sel, finger, VIEW, ROW, 5.0, 1.0).map(|r| r.min)
    }

    #[test]
    fn with_room_on_both_sides_one_row_goes_above_and_a_taller_one_on_the_fingers_side() {
        // One row: above (200 - 40 - 5), even with the finger nearer the bottom; centred on the finger.
        assert_eq!(
            placed((200.0, 220.0), true, pos2(450.0, 218.0), &[]),
            Some(pos2(400.0, 155.0))
        );
        // Taller: the side nearer the finger.
        let tall = (200.0, 300.0);
        assert_eq!(
            placed(tall, false, pos2(450.0, 290.0), &[]),
            Some(pos2(400.0, 305.0))
        );
        assert_eq!(
            placed(tall, false, pos2(450.0, 210.0), &[]),
            Some(pos2(400.0, 155.0))
        );
        // Exactly B + 2m of room counts.
        assert_eq!(
            placed((50.0, 60.0), true, pos2(450.0, 55.0), &[]),
            Some(pos2(400.0, 5.0))
        );
    }

    #[test]
    fn with_room_on_one_side_only_the_row_goes_there() {
        // 30 above: below. 20 below: above, whatever the row count or the finger.
        assert_eq!(
            placed((30.0, 60.0), true, pos2(450.0, 40.0), &[]),
            Some(pos2(400.0, 65.0))
        );
        assert_eq!(
            placed((440.0, 480.0), false, pos2(450.0, 475.0), &[]),
            Some(pos2(400.0, 395.0))
        );
    }

    #[test]
    fn with_room_on_neither_side_the_row_goes_inside_clamped_to_the_viewport() {
        let tall = (20.0, 480.0);
        assert_eq!(
            placed(tall, false, pos2(450.0, 100.0), &[]),
            Some(pos2(400.0, 25.0))
        );
        assert_eq!(
            placed(tall, false, pos2(450.0, 400.0), &[]),
            Some(pos2(400.0, 435.0))
        );
        assert_eq!(
            placed(tall, true, pos2(450.0, 400.0), &[]),
            Some(pos2(400.0, 25.0))
        );
        // Taller than the viewport: the inset edge is out of view, so the row sits at the viewport's edge.
        let taller = (-100.0, 600.0);
        assert_eq!(
            placed(taller, false, pos2(450.0, 100.0), &[]),
            Some(pos2(400.0, 0.0))
        );
        assert_eq!(
            placed(taller, false, pos2(450.0, 400.0), &[]),
            Some(pos2(400.0, 460.0))
        );
    }

    #[test]
    fn the_row_slides_into_the_viewport_and_off_the_handles() {
        let one = (200.0, 220.0);
        // Centred on the finger, slid right (or left) until it is inside.
        assert_eq!(
            placed(one, true, pos2(20.0, 210.0), &[]),
            Some(pos2(0.0, 155.0))
        );
        assert_eq!(
            placed(one, true, pos2(990.5, 210.0), &[]),
            Some(pos2(899.5, 155.0))
        );
        // A handle under the centred row (y 155..195): slid right 1 px at a time until its left end clears it.
        let handle = Rect::from_x_y_ranges(430.0..=470.0, 180.0..=200.0);
        assert_eq!(
            placed(one, true, pos2(450.0, 210.0), &[handle]),
            Some(pos2(470.0, 155.0))
        );
        // No room right of it: then left, until the row's right end clears it.
        let handle = Rect::from_x_y_ranges(900.0..=960.0, 180.0..=200.0);
        assert_eq!(
            placed(one, true, pos2(920.0, 210.0), &[handle]),
            Some(pos2(800.0, 155.0))
        );
        // A handle beside the row (below it here) is no obstacle.
        let handle = Rect::from_x_y_ranges(430.0..=470.0, 195.0..=215.0);
        assert_eq!(
            placed(one, true, pos2(450.0, 210.0), &[handle]),
            Some(pos2(400.0, 155.0))
        );
        // No place clears the handle: the centred row stays (moved into the viewport).
        let wide = Rect::from_x_y_ranges(-10.0..=1010.0, 180.0..=200.0);
        assert_eq!(
            placed(one, true, pos2(450.0, 210.0), &[wide]),
            Some(pos2(400.0, 155.0))
        );
    }

    #[test]
    fn a_selection_off_screen_sideways_holds_the_row_at_the_edge_and_one_off_screen_vertically_hides_it()
     {
        let at = |x: (f32, f32), y: (f32, f32)| {
            let sel = Selection {
                rect: Rect::from_x_y_ranges(x.0..=x.1, y.0..=y.1),
                one_row: true,
                handles: &[],
            };
            place_row(&sel, pos2(500.0, 250.0), VIEW, ROW, 5.0, 1.0).map(|r| r.min)
        };
        assert_eq!(at((-300.0, -200.0), (200.0, 220.0)), Some(pos2(0.0, 155.0)));
        assert_eq!(at((-300.0, 0.0), (200.0, 220.0)), Some(pos2(0.0, 155.0)));
        assert_eq!(
            at((1000.0, 1200.0), (200.0, 220.0)),
            Some(pos2(900.0, 155.0))
        );
        // Below the viewport the row would go above it, out of view; above, below it, out of view.
        assert_eq!(at((400.0, 500.0), (600.0, 620.0)), None);
        assert_eq!(at((400.0, 500.0), (-80.0, -60.0)), None);
        // Partly in view, the needed place in view: shown.
        assert_eq!(at((400.0, 500.0), (480.0, 520.0)), Some(pos2(450.0, 435.0)));
    }

    #[test]
    fn a_viewport_narrower_than_the_row_and_its_margins_hides_it() {
        let sel = Selection {
            rect: Rect::from_x_y_ranges(40.0..=60.0, 200.0..=220.0),
            one_row: true,
            handles: &[],
        };
        let view = |w: f32| Rect::from_min_size(Pos2::ZERO, vec2(w, 500.0));
        let place = |w| place_row(&sel, pos2(50.0, 210.0), view(w), ROW, 5.0, 1.0);
        assert_eq!(place(109.9), None);
        assert_eq!(place(110.0).map(|r| r.min), Some(pos2(0.0, 155.0)));
    }

    const GRID: GridView = GridView {
        grid: VIEW,
        map: [0.0, 0.0, 64.0, 16.0],
    };
    const SEL: Rect = Rect::from_min_max(pos2(400.0, 200.0), pos2(500.0, 220.0));

    /// A tap (press and release in one frame) at `p` with the selection `sel`.
    fn tap(row: &mut FloatingRow, view: GridView, sel: Option<Rect>, p: Pos2) {
        row.track(view, sel, Some(p), Some(p));
    }

    #[test]
    fn the_row_shows_after_a_tap_on_the_selection_and_hides_on_a_scroll_or_zoom() {
        let mut row = FloatingRow::default();
        row.track(GRID, Some(SEL), None, None);
        assert_eq!(row.anchor, None, "a selection alone shows nothing");
        // A tap off the selection, which keeps it, shows nothing either.
        tap(&mut row, GRID, Some(SEL), pos2(100.0, 100.0));
        assert_eq!(row.anchor, None);
        tap(&mut row, GRID, Some(SEL), pos2(420.0, 210.0));
        assert_eq!(row.anchor, Some(pos2(420.0, 210.0)));
        row.track(GRID, Some(SEL), None, None);
        assert_eq!(
            row.anchor,
            Some(pos2(420.0, 210.0)),
            "kept while the view stays"
        );
        // A scroll, a zoom and a new grid rect each hide it, the selection still in view.
        for view in [
            GridView {
                map: [0.0, -1.0, 64.0, 16.0],
                ..GRID
            },
            GridView {
                map: [0.0, 0.0, 64.0, 17.0],
                ..GRID
            },
            GridView {
                grid: VIEW.with_max_x(900.0),
                ..GRID
            },
        ] {
            tap(&mut row, GRID, Some(SEL), pos2(420.0, 210.0));
            row.track(view, Some(SEL), None, None);
            assert_eq!(row.anchor, None, "{view:?}");
            row.track(view, Some(SEL), None, None);
            assert_eq!(row.anchor, None, "{view:?}: it stays hidden");
            row.track(GRID, Some(SEL), None, None);
        }
    }

    #[test]
    fn a_press_that_scrolls_or_starts_on_the_row_re_shows_nothing() {
        let mut row = FloatingRow::default();
        // A pan from the selection: the view changes before the release.
        row.track(GRID, Some(SEL), Some(pos2(420.0, 210.0)), None);
        let panned = GridView {
            map: [-30.0, 0.0, 64.0, 16.0],
            ..GRID
        };
        row.track(panned, Some(SEL), None, Some(pos2(420.0, 210.0)));
        assert_eq!(row.anchor, None);
        // A press on the drawn row: its button moves the selection (Copy), and the row stays where it was.
        tap(&mut row, panned, Some(SEL), pos2(420.0, 210.0));
        row.drawn = Some(Rect::from_min_size(pos2(370.0, 155.0), ROW));
        let moved = SEL.translate(vec2(100.0, 0.0));
        row.track(panned, Some(SEL), Some(pos2(380.0, 170.0)), None);
        row.track(panned, Some(moved), None, Some(pos2(380.0, 170.0)));
        assert_eq!(row.anchor, Some(pos2(420.0, 210.0)));
    }

    #[test]
    fn a_gesture_that_makes_the_selection_shows_the_row_at_its_release() {
        let mut row = FloatingRow::default();
        // A box from empty space: nothing selected at the press, the release off what it selected.
        row.track(GRID, None, Some(pos2(300.0, 100.0)), None);
        row.track(GRID, None, None, None);
        row.track(GRID, Some(SEL), None, Some(pos2(600.0, 300.0)));
        assert_eq!(row.anchor, Some(pos2(600.0, 300.0)));
        // Losing the selection hides it; a new one's first view is no scroll.
        row.track(GRID, None, None, None);
        assert_eq!(row.anchor, None);
        let other = GridView {
            map: [5.0, 5.0, 64.0, 16.0],
            ..GRID
        };
        row.track(other, None, Some(pos2(450.0, 210.0)), None);
        row.track(other, Some(SEL), None, Some(pos2(450.0, 210.0)));
        assert_eq!(row.anchor, Some(pos2(450.0, 210.0)));
    }

    #[test]
    fn select_all_shows_the_row_at_the_viewports_centre() {
        let mut row = FloatingRow::default();
        row.track(GRID, Some(SEL), None, None);
        row.select_all();
        assert_eq!(row.anchor, Some(pos2(500.0, 250.0)));
    }

    // ---- the clip drawing and the muted clip's tint ----

    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::song::{ChannelKind, SynthKind};
    use crate::theme::theme_by_id;

    fn rgba(c: u32) -> Rgba {
        Rgba::hex(c)
    }

    #[test]
    fn the_tint_runs_from_the_background_through_the_content_colour_at_0_66_toward_the_text() {
        let color = rgba(0x40A0FF);
        let close = |a: Rgba, b: Rgba| a.to_color32() == b.to_color32();
        // Classic Gray is dark: k = 0.75, content = colour × 0.65.
        let theme = theme_by_id(0);
        let (bg, text) = (theme.colors.background, theme.colors.text);
        let (content, stops) = tint(&theme, color);
        let b = theme.options.playlist_track_note_brightness;
        assert_eq!(b, 0.65);
        assert!(close(content, Rgba::rgb(color.r * b, color.g * b, b)));
        assert!(close(stops[0], mix(bg, content, 0.75)));
        assert_eq!(stops[1], content);
        assert!(close(stops[2], mix(content, text, 0.4)));
        // The middle stop at 0.66: 0.8 is 14/34 of the way from it to the top.
        assert_eq!(sample(&stops, 0.0), stops[0]);
        assert!(close(sample(&stops, 0.66), content));
        assert!(close(
            sample(&stops, 0.8),
            mix(content, stops[2], 0.14 / 0.34)
        ));
        assert!(close(sample(&stops, 1.0), stops[2]));
        // Paper's background is brighter than its text: light, k = 0.25.
        let paper = theme_by_id(3);
        let (content, stops) = tint(&paper, color);
        assert!(close(stops[0], mix(paper.colors.background, content, 0.25)));
        // A text colour of lightness ≤ 0.25 divides by the brightness instead (clamped to 1).
        let mut dark_text = theme_by_id(0);
        dark_text.colors.text = rgba(0x202020);
        let (content, _) = tint(&dark_text, color);
        assert!(close(content, Rgba::rgb(color.r / b, color.g / b, 1.0)));
    }

    type Texts = Vec<(String, Color32)>;
    type Paths = Vec<(Vec<Pos2>, Color32)>;

    /// The test clips' track row less the device pixel of gap under it, Y_r 100 and Y_{r+1} 164: at ppp 1 (U 32,
    /// t 2) the clip box spans y 101 ..= 161, the name strip 101 ..= 123.4 and the miniature 123.4 ..= 149.
    const TRACK: Rect = Rect {
        min: pos2(0.0, 100.0),
        max: pos2(2000.0, 163.0),
    };
    /// 32 points a beat from x = 100.
    const AXIS: TimeAxis = TimeAxis {
        origin: 100.0,
        beat_w: 32.0,
    };
    /// A grid over everything.
    const ALL: Rect = Rect::EVERYTHING;

    fn m1() -> Metrics {
        Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE)
    }

    /// A song with a MiniSynth (channel 1), whose colour and name the test clips take.
    fn synth() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song
    }

    /// Every shape [`paint_clip`] paints for `lane`'s first clip with `look` (then a current clip's
    /// [`paint_outline`]), on `theme` at ppp 1 and `axis` on [`TRACK`], its painter clipped to `grid`, in paint order;
    /// and the clip box.
    fn paint_all(
        lane: &Lane,
        wave: Option<(&Waveform, Wave)>,
        look: Look,
        theme: &Theme,
        grid: Rect,
        axis: TimeAxis,
    ) -> (Vec<egui::Shape>, Rect) {
        paint_at(lane, wave, look, theme, grid, axis, &m1())
    }

    /// [`paint_all`] at the metrics `m`.
    fn paint_at(
        lane: &Lane,
        wave: Option<(&Waveform, Wave)>,
        look: Look,
        theme: &Theme,
        grid: Rect,
        axis: TimeAxis,
        m: &Metrics,
    ) -> (Vec<egui::Shape>, Rect) {
        let (clip, song) = (&lane.clips[0], synth());
        let cx = UiCtx { theme, m };
        let b = clip_box(clip_rect(clip, TRACK, axis), m);
        let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
            let panel = egui::CentralPanel::default().frame(egui::Frame::NONE);
            panel.show(ctx, |ui| {
                let painter = ui.painter().with_clip_rect(grid);
                let channel = &song.channels[1];
                paint_clip(&painter, &cx, channel, lane, clip, wave, look, b, axis);
                if look.current {
                    paint_outline(&painter, &cx, b, look.dimmed);
                }
            });
        });
        // Less the panel's empty placeholder.
        let empty = |s: &egui::Shape| matches!(s, egui::Shape::Rect(r) if r.fill == Color32::TRANSPARENT && r.stroke.width == 0.0);
        let shapes = out.shapes.into_iter().map(|c| c.shape);
        (shapes.filter(|s| !empty(s)).collect(), b)
    }

    /// [`paint_all`] on theme 0 over everything at [`AXIS`]: the filled rects inside the clip box with their fill (the
    /// body first), the texts with their colour, then the filled paths.
    fn painted_with(
        lane: &Lane,
        wave: Option<(&Waveform, Wave)>,
        look: Look,
    ) -> (Vec<(Rect, Color32)>, Texts, Paths) {
        let (shapes, b) = paint_all(lane, wave, look, &theme_by_id(0), ALL, AXIS);
        let (mut fills, mut texts, mut paths) = (Vec::new(), Vec::new(), Vec::new());
        for shape in shapes {
            match shape {
                egui::Shape::Rect(r) if b.contains_rect(r.rect) => fills.push((r.rect, r.fill)),
                egui::Shape::Text(t) => {
                    let color = t.galley.job.sections[0].format.color;
                    texts.push((t.galley.text().to_owned(), color));
                }
                egui::Shape::Path(p) => paths.push((p.points, p.fill)),
                _ => {}
            }
        }
        (fills, texts, paths)
    }

    fn painted(lane: &Lane) -> (Vec<(Rect, Color32)>, Texts, Paths) {
        painted_with(lane, None, Look::default())
    }

    /// The waveform columns `lane`'s clip shows: the quads of its mesh (`audio_editor::add_column`) in the clip box,
    /// with their colours. No `Shape::Rect` counts: egui shrinks a 1 px tall one to nothing.
    fn wave_quads(lane: &Lane, wave: (&Waveform, Wave), look: Look) -> Vec<(Rect, Color32)> {
        let (shapes, b) = paint_all(lane, Some(wave), look, &theme_by_id(0), ALL, AXIS);
        let quads = audio_editor::mesh_quads(&shapes).into_iter();
        quads.filter(|q| b.contains_rect(q.0)).collect()
    }

    /// The content colour and tint stops of the test clips on theme 0.
    fn stops() -> (Rgba, [Rgba; 3]) {
        let theme = theme_by_id(0);
        tint(&theme, channel_color(synth().channels[1].hue, &theme))
    }

    /// A notes lane with a 4-beat clip at tick 0 shown `visible` beats, its notes on beats 0, 1 and 2 at velocities 0,
    /// the default and 255, keys 60, 61 and 62.
    fn notes_lane(visible: f64) -> Lane {
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 4.0);
        clip.visible_len_beats = visible;
        clip.events = (0..3)
            .map(|i| Event::Note {
                pos: i * 128,
                len_beats: 0.5,
                key: 60 + i as u8,
                velocity: VELOCITIES[i as usize],
                pan: Event::DEFAULT_PAN,
                slide: false,
            })
            .collect();
        lane
    }

    const VELOCITIES: [u8; 3] = [0, Event::DEFAULT_VELOCITY, 255];

    #[test]
    fn the_clip_box_runs_from_y_r_plus_a_pixel_to_t_and_a_pixel_above_the_next_row_at_least_t_wide()
    {
        // ppp 1, t 2: x truncated, y0 = 100 + 1, y1 = 164 − 2 − 1.
        let m = m1();
        let b = clip_box(Rect::from_x_y_ranges(100.7..=228.9, 100.0..=163.0), &m);
        assert_eq!(
            b,
            Rect::from_min_max(pos2(100.0, 101.0), pos2(228.0, 161.0))
        );
        let narrow = clip_box(Rect::from_x_y_ranges(100.2..=100.9, 100.0..=163.0), &m);
        assert_eq!(narrow.x_range(), Rangef::new(100.0, 102.0));
        // ppp 2: device pixels are half points, and the row less its gap ends half a point above the next.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        let b = clip_box(Rect::from_x_y_ranges(100.7..=228.9, 100.0..=163.5), &m);
        let want = Rect::from_min_max(pos2(100.5, 100.5), pos2(228.5, 164.0 - m.t() - 0.5));
        assert_eq!(b, want);
    }

    #[test]
    fn only_the_current_clip_has_the_light_frame_1_5_t_on_every_side() {
        let lane = notes_lane(4.0);
        let mut theme = theme_by_id(0);
        let (bg, text) = (theme.colors.background, theme.colors.text);
        let (t, r) = (m1().t(), m1().u(CORNER));
        assert_eq!(t, 2.0);
        let rects = |shapes: Vec<egui::Shape>| -> Vec<egui::epaint::RectShape> {
            (shapes.into_iter())
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) => Some(r),
                    _ => None,
                })
                .collect()
        };
        // Not current: nothing past the frame, and no outline.
        let (shapes, b) = paint_all(&lane, None, Look::default(), &theme, ALL, AXIS);
        let plain = rects(shapes);
        assert!(plain.iter().all(|r| b.expand(1.0).contains_rect(r.rect)));
        assert!(plain.iter().all(|r| r.stroke.width == 0.0));
        // Current: F, the box grown 1.5t on every side with corners r + 1.5t, filled `background` then `text` at α a
        // (a quarter of it when dimmed); then the clip as before; last F's outline, t + 1 px wide, at α 1 − a² while
        // a² < 0.9, else a.
        for (a, dimmed, fill, edge) in [
            (1.0, false, 1.0, 1.0),
            (0.5, false, 0.5, 0.75),
            (0.92, false, 0.92, 1.0 - 0.92 * 0.92),
            (0.96, false, 0.96, 0.96),
            (1.0, true, 0.25, 1.0 - 0.25 * 0.25),
        ] {
            theme.options.playlist_track_background_transparency = a;
            let look = Look {
                current: true,
                dimmed,
                ..Look::default()
            };
            let (shapes, b) = paint_all(&lane, None, look, &theme, ALL, AXIS);
            let lit = rects(shapes);
            let f = b.expand(1.5 * t);
            let corner = egui::CornerRadius::from(r + 1.5 * t);
            assert_eq!(
                (lit[0].rect, lit[0].corner_radius, lit[0].fill),
                (f, corner, bg.to_color32())
            );
            let fill = Rgba { a: fill, ..text }.to_color32();
            assert_eq!((lit[1].rect, lit[1].fill), (f, fill), "a {a}");
            assert_eq!(lit[2].rect, b.expand(1.0));
            let outline = lit.last().unwrap();
            let stroke = Stroke::new(t + 1.0, Rgba { a: edge, ..text }.to_color32());
            assert_eq!((outline.rect, outline.corner_radius), (f, corner), "a {a}");
            assert_eq!(
                (outline.stroke, outline.stroke_kind),
                (stroke, StrokeKind::Middle),
                "a {a}"
            );
        }
    }

    #[test]
    fn the_frame_is_background_at_0_3_a_pixel_around_the_clip_box_under_the_body() {
        let theme = theme_by_id(0);
        let shade = Rgba {
            a: 0.3,
            ..theme.colors.background
        };
        // The 4-beat clip's box: x 100 … 228; y 101 … 161 at ppp 1, 100.5 … 161 at ppp 2 (t = 2 points either way).
        for (ppp, box_) in [
            (
                1.0,
                Rect::from_min_max(pos2(100.0, 101.0), pos2(228.0, 161.0)),
            ),
            (
                2.0,
                Rect::from_min_max(pos2(100.0, 100.5), pos2(228.0, 161.0)),
            ),
        ] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
            let lane = notes_lane(4.0);
            let (shapes, b) = paint_at(&lane, None, Look::default(), &theme, ALL, AXIS, &m);
            let [egui::Shape::Rect(frame), egui::Shape::Rect(body), ..] = &shapes[..] else {
                panic!("{shapes:?}");
            };
            assert_eq!(b, box_, "ppp {ppp}");
            let grown = b.expand(1.0 / ppp);
            assert_eq!(
                (frame.rect, frame.fill),
                (grown, shade.to_color32()),
                "ppp {ppp}"
            );
            assert_eq!(body.rect, b, "ppp {ppp}");
        }
    }

    #[test]
    fn the_body_is_the_channel_colour_at_the_track_transparency_and_has_no_header_strip() {
        let mut theme = theme_by_id(0);
        theme.options.playlist_track_background_transparency = 0.5;
        let (shapes, b) = paint_all(&notes_lane(4.0), None, Look::default(), &theme, ALL, AXIS);
        let fills: Vec<(Rect, Color32)> = (shapes.iter())
            .filter_map(|s| match s {
                egui::Shape::Rect(r) => Some((r.rect, r.fill)),
                _ => None,
            })
            .collect();
        let color = channel_color(synth().channels[1].hue, &theme);
        assert_eq!(fills[1], (b, Rgba { a: 0.5, ..color }.to_color32()));
        // The "header" is only the body's top 0.7 U: past the frame and the body, every fill is a note below it.
        let strip = b.top() + 0.7 * 32.0;
        assert!(fills.len() > 2);
        assert!(
            fills[2..].iter().all(|(r, _)| r.top() >= strip),
            "{fills:?}"
        );
    }

    /// The name [`paint_clip`] writes on an automation clip labelled `label`, shown `visible` beats on a grid from x
    /// `left`: its text, where its galley sits, its colour and font.
    fn name(label: &str, visible: f64, left: f32) -> Option<(String, Rect, Color32, FontId)> {
        name_at(label, visible, left, &m1())
    }

    /// [`name`] at the metrics `m`.
    fn name_at(
        label: &str,
        visible: f64,
        left: f32,
        m: &Metrics,
    ) -> Option<(String, Rect, Color32, FontId)> {
        let mut lane = automation_lane(label, "");
        lane.clips[0].visible_len_beats = visible;
        let grid = Rect::from_x_y_ranges(left..=2000.0, 0.0..=2000.0);
        let theme = theme_by_id(0);
        let (shapes, _) = paint_at(&lane, None, Look::default(), &theme, grid, AXIS, m);
        shapes.into_iter().find_map(|s| match s {
            egui::Shape::Text(t) => {
                let format = &t.galley.job.sections[0].format;
                let rect = Rect::from_min_size(t.pos, t.galley.size());
                Some((
                    t.galley.text().to_owned(),
                    rect,
                    format.color,
                    format.font_id.clone(),
                ))
            }
            _ => None,
        })
    }

    #[test]
    fn the_name_is_8_pt_in_the_content_colour_from_5_px_in_centred_in_0_7_u_with_a_middle_ellipsis()
    {
        let m = m1();
        let (content, _) = stops();
        // 8 pt, from x0 + 5 px, centred in the body's top 0.7 U (101 ..= 123.4).
        let (text, at, color, font) = name("Drive : Distortion", 8.0, 0.0).unwrap();
        assert_eq!(text, "Drive : Distortion");
        assert_eq!(
            (color, font),
            (content.to_color32(), m.font_id(FontSize::Pt8))
        );
        assert_eq!(at.left(), 105.0);
        assert!((at.center().y - 112.2).abs() < 0.01, "{at:?}");
        // At ppp 2 the 5 px are 2.5 points, from y0 = 100.5.
        let m2 = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
        let (_, at, _, font) = name_at("Drive : Distortion", 8.0, 0.0, &m2).unwrap();
        assert_eq!((at.left(), font), (102.5, m2.font_id(FontSize::Pt8)));
        assert!((at.center().y - 111.7).abs() < 0.01, "{at:?}");
        // Too long for x0 + 5 px … x1 − 2 px (8 beats: 249 px): a middle ellipsis keeping the last 5 characters.
        let long = "Cutoff Frequency : Analog Filter Bank";
        let (text, ..) = name(long, 8.0, 0.0).unwrap();
        let (head, tail) = text.split_once("...").expect(&text);
        assert_eq!(tail, " Bank");
        assert!(long.starts_with(head) && head.len() > 5, "{text}");
        // Scrolled out to the left: from the grid's left edge.
        assert_eq!(name("Level", 8.0, 150.0).unwrap().1.left(), 150.0);
        // No room (6 px: x1 − 2 < x0 + 5): no name.
        assert_eq!(name("Level", 6.0 / 32.0, 0.0), None);
    }

    #[test]
    fn each_loop_repeat_from_the_second_has_a_background_triangle_down_at_its_start() {
        // A 2-beat loop shown 6 beats from 2.5 beats into its content: o = 0.5 beat (wrapped), so repeats 0 … ⌈6.5 / 2⌉
        // − 1 = 3 start at x 84, 148, 212 and 276; T = y0 = 101, 2t = 4.
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 2.0);
        (clip.visible_len_beats, clip.offset_beats) = (6.0, 2.5);
        let bg = theme_by_id(0).colors.background.to_color32();
        let tri = |x: f32| {
            let points = vec![pos2(x - 4.0, 101.0), pos2(x + 4.0, 101.0), pos2(x, 105.0)];
            (points, bg)
        };
        assert_eq!(painted(&lane).2, [tri(148.0), tri(212.0), tri(276.0)]);
        // One repeat: none.
        lane.clips[0].visible_len_beats = 1.5;
        assert!(painted(&lane).2.is_empty());
    }

    #[test]
    fn a_dimmed_repeat_draws_its_miniature_at_the_lowest_stop_and_nothing_else_differently() {
        // The notes of two repeats (x 100 … 228 … 356) by velocity; the body and name as they are.
        let lane = notes_lane(8.0);
        let (_, stops) = stops();
        let ink = |v: u8| sample(&stops, f32::from(v) / 255.0).to_color32();
        let low = stops[0].to_color32();
        let bright: Vec<Color32> = VELOCITIES.map(ink).to_vec();
        assert!(bright[0] != bright[1] && bright[1] != bright[2]);
        let inks = |look: Look| {
            let (fills, texts, _) = painted_with(&lane, None, look);
            let inks: Vec<Color32> = fills[1..].iter().map(|f| f.1).collect();
            (fills[0], texts, inks)
        };
        let (body, texts, all) = inks(Look::default());
        assert_eq!(all, [bright.clone(), bright.clone()].concat());
        // Every repeat (a muted clip, or another while Preview plays): stop 0 whatever the velocity.
        let dimmed = Look {
            dimmed: true,
            ..Look::default()
        };
        assert_eq!(inks(dimmed), (body, texts.clone(), vec![low; 6]));
        // Selected with its handles on repeat 1, or on repeat 0: the other one.
        let picked = |i| Look {
            picked: Some(i),
            ..Look::default()
        };
        assert_eq!(inks(picked(1)).2, [vec![low; 3], bright.clone()].concat());
        assert_eq!(inks(picked(0)).2, [bright, vec![low; 3]].concat());
        assert_eq!(inks(picked(1)).0, body);
    }

    #[test]
    fn a_notes_bar_is_t_tall_at_its_key_over_the_span_plus_2_from_its_start_cut_at_its_repeats_end()
    {
        // Keys 60 … 64 in the 25.6 px miniature (123.4 ..= 149): p = 25.6 / 6, y = ⌊149 − 2 − (key − 59)·p⌋. Widths:
        // half a beat (16 px), a tick (2 px at least), and two beats from beat 3, cut at the loop's end (beat 4).
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 4.0);
        clip.visible_len_beats = 8.0;
        let note = |pos, len_beats, key| Event::Note {
            pos,
            len_beats,
            key,
            velocity: 100,
            pan: Event::DEFAULT_PAN,
            slide: false,
        };
        clip.events = vec![note(0, 0.5, 60), note(128, 0.001, 64), note(384, 2.0, 62)];
        let bars = |x: f32| {
            [(0.0, 16.0, 142.0), (32.0, 2.0, 125.0), (96.0, 32.0, 134.0)]
                .map(|(dx, w, y)| Rect::from_min_size(pos2(x + dx, y), vec2(w, 2.0)))
        };
        let want = [bars(100.0), bars(228.0)].concat();
        let got: Vec<Rect> = painted(&lane).0[1..].iter().map(|f| f.0).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_step_lanes_hits_are_2t_or_a_quarter_beat_wide_keyed_by_pad_and_a_free_grids_by_length() {
        // Pads 0 and 2: p = 25.6 / 4, y = ⌊147 − (pad + 1)·p⌋. A step lane draws max(2t, ¼ beat): 8 px at 32 points
        // a beat, 4 px (2t) at 8.
        let mut lane = Lane::new(LaneKind::Drums);
        let clip = lane.add_clip(0, 4.0);
        let hit = |pos, pad| Event::Drum {
            pos,
            len_beats: 0.001,
            pad,
            velocity: 100,
            pan: Event::DEFAULT_PAN,
            value: 0x7FFF,
            repeat: 0,
        };
        clip.events = vec![hit(0, 0), hit(128, 2)];
        let theme = theme_by_id(0);
        let bars = |lane: &Lane, beat_w: f32| -> Vec<Rect> {
            let axis = TimeAxis {
                origin: 100.0,
                beat_w,
            };
            let (shapes, b) = paint_all(lane, None, Look::default(), &theme, ALL, axis);
            (shapes.into_iter())
                .filter_map(|s| match s {
                    egui::Shape::Rect(r) if r.rect.height() == 2.0 && b.contains_rect(r.rect) => {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .collect()
        };
        let bar = |x: f32, y: f32, w: f32| Rect::from_min_size(pos2(x, y), vec2(w, 2.0));
        assert_eq!(lane.grid, DrumGrid::Steps);
        assert_eq!(
            bars(&lane, 32.0),
            [bar(100.0, 140.0, 8.0), bar(132.0, 127.0, 8.0)]
        );
        assert_eq!(
            bars(&lane, 8.0),
            [bar(100.0, 140.0, 4.0), bar(108.0, 127.0, 4.0)]
        );
        // A free-grid lane: its length, at least 2 px.
        lane.grid = DrumGrid::Notes;
        assert_eq!(
            bars(&lane, 32.0),
            [bar(100.0, 140.0, 2.0), bar(132.0, 127.0, 2.0)]
        );
    }

    #[test]
    fn the_miniature_paints_only_the_plays_that_reach_the_view() {
        // A hundred passes of a one-beat loop with a half-beat note, 32 points a beat; the view is beats 10 to 14.
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 1.0);
        clip.visible_len_beats = 100.0;
        clip.events = vec![note(0, 60)];
        let axis = TimeAxis {
            origin: 0.0,
            beat_w: 32.0,
        };
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let area = Rect::from_x_y_ranges(0.0..=3200.0, 0.0..=48.0);
        let view = Rect::from_x_y_ranges(320.0..=448.0, 0.0..=48.0);
        let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let painter = ui.painter().with_clip_rect(view);
                notes(&painter, clip, false, area, axis, |_| Color32::RED, &m);
            });
        });
        let xs: Vec<f32> = (out.shapes.iter())
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect.left()),
                _ => None,
            })
            .collect();
        assert_eq!(xs, [320.0, 352.0, 384.0, 416.0]);
    }

    /// The red rects `notes` paints of `clip` in `area` (`view` its visible part), on its own thread: `None` when
    /// that takes more than 5 s.
    fn miniature_fills(
        clip: Clip,
        area: Rect,
        view: Rect,
        axis: TimeAxis,
        m: Metrics,
    ) -> Option<Vec<Rect>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let painter = ui.painter().with_clip_rect(view);
                    notes(&painter, &clip, false, area, axis, |_| Color32::RED, &m);
                });
            });
            let fills = (out.shapes.iter())
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if r.fill == Color32::RED => Some(r.rect),
                    _ => None,
                })
                .collect();
            let _ = tx.send(fills);
        });
        rx.recv_timeout(std::time::Duration::from_secs(5)).ok()
    }

    #[test]
    fn the_miniature_looks_back_a_loop_pass_for_long_notes() {
        // A 4-tick loop over a million beats with a note a million beats long, scrolled to beat 500 000: looking back
        // by the note's length walked every pass from the clip start, 1.6e7 a frame.
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 4.0 / f64::from(PPQ));
        clip.visible_len_beats = 1e6;
        clip.events = vec![Event::Note {
            pos: 0,
            len_beats: 1e6,
            key: 60,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }];
        let axis = TimeAxis {
            origin: -16_000_000.0,
            beat_w: 32.0,
        };
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let area = Rect::from_x_y_ranges(axis.x(0.0)..=axis.x(1.28e8), 0.0..=48.0);
        let view = Rect::from_x_y_ranges(0.0..=128.0, 0.0..=48.0);
        let fills = (miniature_fills(clip.clone(), area, view, axis, m).map(|f| f.len()))
            .expect("painted within 5 s");
        // The view and one pass before it: 516 ticks, a play every 4.
        assert!((120..=140).contains(&fills), "{fills}");
        // A 64-beat clip with one 64-beat note (a held note as a take records), scrolled to beats 40 to 44: the note
        // starts 40 beats before the view, ten views' widths, and is still drawn across it.
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 64.0);
        clip.visible_len_beats = 64.0;
        clip.events = vec![Event::Note {
            pos: 0,
            len_beats: 64.0,
            key: 60,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }];
        let axis = TimeAxis {
            origin: -1280.0,
            beat_w: 32.0,
        };
        let area = Rect::from_x_y_ranges(axis.x(0.0)..=axis.x(64.0 * f64::from(PPQ)), 0.0..=48.0);
        let fills = miniature_fills(clip.clone(), area, view, axis, m).expect("painted within 5 s");
        assert!(
            fills.iter().any(|r| r.left() < 0.0 && r.right() > 128.0),
            "{fills:?}"
        );
    }

    #[test]
    fn a_one_bar_loop_of_200_notes_paints_at_the_lowest_zoom_on_every_ui_scale() {
        // The playlist's lowest zoom: d/48 U a beat, at d = 4.
        for scale in [60, 75, 90, 100, 110, 125, 150, 175, 200] {
            let m = Metrics::from_pixels_per_point(1.0, scale);
            let mut lane = Lane::new(LaneKind::Notes);
            let clip = lane.add_clip(0, 4.0);
            clip.visible_len_beats = 64.0;
            clip.events = (0..200).map(|i| note(i * 2, 60)).collect();
            let axis = TimeAxis {
                origin: 0.0,
                beat_w: m.u(4.0 / 48.0),
            };
            let area = Rect::from_x_y_ranges(0.0..=axis.x(64.0 * 128.0), 0.0..=48.0);
            // Every pass paints its notes, from the first to the last: a note under one painted before it is not
            // painted again.
            let fills = miniature_fills(clip.clone(), area, area, axis, m).unwrap();
            for pass in 0..16 {
                let (first, last) = (pass * 4 * PPQ, pass * 4 * PPQ + 199 * 2);
                let (a, b) = (axis.x(f64::from(first)), axis.x(f64::from(last)));
                let covered = |x: f32| fills.iter().any(|r| r.left() <= x.floor() && x < r.right());
                assert!(covered(a) && covered(b), "scale {scale}, pass {pass}");
            }
        }
    }

    #[test]
    fn a_loop_narrower_than_a_pixel_paints_no_miniature_and_does_not_unlink() {
        // A one-tick loop, a quarter point wide at 32 points a beat: one play per tick would be many per pixel.
        let mut lane = Lane::new(LaneKind::Notes);
        let clip = lane.add_clip(0, 1.0 / f64::from(PPQ));
        clip.visible_len_beats = 100.0;
        clip.events = vec![note(0, 60)];
        let axis = TimeAxis {
            origin: 0.0,
            beat_w: 32.0,
        };
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let area = Rect::from_x_y_ranges(0.0..=3200.0, 0.0..=48.0);
        assert_eq!(
            miniature_fills(clip.clone(), area, area, axis, m).map(|f| f.len()),
            Some(0)
        );
        // Nor repeats: no triangles, no notes.
        let (fills, _, paths) = painted(&lane);
        assert_eq!((fills.len(), paths.len()), (1, 0));
        // A loop under a tick is no loop: nothing to unlink.
        let clip = &mut lane.clips[0];
        clip.loop_len_beats = 1e-9;
        assert!(!loops(clip));
    }

    #[test]
    fn an_audio_clip_is_named_after_its_sample_and_draws_its_waveform_a_column_a_pixel_at_0_8() {
        // The clip's name is the file's; the audio editor's waveform routine over the miniature (123.4 ..= 149), in
        // the tint at 0.8, at stop 0 when dimmed. 64 frames a beat, 32 points a beat: two frames a column.
        let (content, stops) = stops();
        let mut lane = Lane::new(LaneKind::Audio);
        lane.add_clip(0, 2.0).file.sample = Some(Box::new(crate::song::SamplerSlot {
            name: "Loop 01".into(),
            ..Default::default()
        }));
        // ±0.5 the first beat, ±0.25 the second.
        let left: Vec<f32> = (0..128)
            .map(|i| if i < 64 { 0.5 } else { 0.25 } * if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let data = SampleData {
            sample_rate: 8000,
            left: left.into(),
            right: None,
            ..Default::default()
        };
        let wave = |reversed| Wave {
            frames: 128,
            per_beat: 64.0,
            reversed,
        };
        let data = Waveform::new(Arc::new(data));
        let columns = |lane: &Lane, reversed, look, ink: Color32| -> (Vec<(f32, Rangef)>, Texts) {
            let (_, texts, _) = painted_with(lane, Some((&data, wave(reversed))), look);
            let quads = wave_quads(lane, (&data, wave(reversed)), look);
            let cols = (quads.iter().filter(|f| f.1 == ink))
                .map(|(r, _)| {
                    assert_eq!(r.width(), 1.0);
                    (r.left(), r.y_range())
                })
                .collect();
            (cols, texts)
        };
        let area = Rect::from_x_y_ranges(0.0..=1.0, 101.0 + 0.7 * 32.0..=149.0);
        let [(centre, scale)] = wave_rows(area, false)[..] else {
            panic!()
        };
        let loud = |x: f32| (x, audio_editor::column_span(-0.5, 0.5, centre, scale, 1.0));
        let soft = |x: f32| {
            (
                x,
                audio_editor::column_span(-0.25, 0.25, centre, scale, 1.0),
            )
        };
        let ink = sample(&stops, 0.8).to_color32();
        let plain = Look::default();
        // 64 columns, x 100 … 163.
        let (cols, _) = columns(&lane, false, plain, ink);
        let want: Vec<_> = (0..64)
            .map(|i| 100.0 + i as f32)
            .map(|x| if x < 132.0 { loud(x) } else { soft(x) })
            .collect();
        assert_eq!(cols, want);
        // Reversed: mirrored.
        let (cols, _) = columns(&lane, true, plain, ink);
        assert_eq!((cols[0], cols[63]), (soft(100.0), loud(163.0)));
        // Shown twice over: each loop pass from its start.
        lane.clips[0].visible_len_beats = 4.0;
        let (cols, texts) = columns(&lane, false, plain, ink);
        assert_eq!(texts, [("Loop 01".to_owned(), content.to_color32())]);
        assert_eq!(cols.len(), 128);
        assert_eq!(
            (cols[63], cols[64], cols[96]),
            (soft(163.0), loud(164.0), soft(196.0))
        );
        // Dimmed: stop 0.
        let dimmed = Look {
            dimmed: true,
            ..plain
        };
        let (cols, _) = columns(&lane, false, dimmed, stops[0].to_color32());
        assert_eq!(cols.len(), 128);
        // No sample to draw: the name alone; no name: the channel's.
        let (fills, texts, _) = painted(&lane);
        assert_eq!((fills.len(), texts.len()), (1, 1));
        lane.clips[0].file.sample = None;
        assert_eq!(painted(&lane).1[0].0, synth().channels[1].name);
    }

    #[test]
    fn a_long_samples_miniature_reads_its_summary_not_its_frames() {
        // 2^20 silent frames under a summary saying ±0.5: a scan of the frames would draw the centre line. 32 768
        // frames a beat at 32 points a beat: 1024 frames, four summary pairs, a column.
        let frames = 1 << 20;
        let data = Arc::new(SampleData {
            sample_rate: 44_100,
            left: vec![0.0; frames].into(),
            right: None,
            ..Default::default()
        });
        let summary = vec![(-0.5, 0.5); frames / honeydew_audio::sample_edit::SUMMARY_FRAMES];
        let source = Waveform {
            data,
            summary: vec![summary].into(),
        };
        let wave = Wave {
            frames,
            per_beat: 32_768.0,
            reversed: false,
        };
        let mut lane = Lane::new(LaneKind::Audio);
        lane.add_clip(0, 2.0).file.sample = Some(Box::default());
        let ink = sample(&stops().1, CURVE_TINT).to_color32();
        let quads = wave_quads(&lane, (&source, wave), Look::default());
        let cols: Vec<_> = (quads.iter().filter(|f| f.1 == ink))
            .map(|(r, _)| r.y_range())
            .collect();
        let area = Rect::from_x_y_ranges(0.0..=1.0, 101.0 + 0.7 * 32.0..=149.0);
        let (centre, scale) = wave_rows(area, false)[0];
        assert_eq!(
            cols,
            [audio_editor::column_span(-0.5, 0.5, centre, scale, 1.0); 64]
        );
    }

    /// An automation lane named `label` (custom `custom`) with one clip: a 4-beat loop shown twice, points at beats
    /// 0, 2 and 3 with values 0, 1 and ½.
    fn automation_lane(label: &str, custom: &str) -> Lane {
        let mut lane = Lane::new(LaneKind::Automation);
        lane.label = label.into();
        lane.file.custom_label = custom.into();
        let clip = lane.add_clip(0, 4.0);
        clip.visible_len_beats = 8.0;
        clip.events = [(0, 0.0), (256, 1.0), (384, 0.5)]
            .map(|(pos, value)| Event::Automation {
                pos,
                value,
                control: true,
            })
            .to_vec();
        lane
    }

    #[test]
    fn an_automation_clip_is_a_stepped_area_from_each_point_to_the_next_under_its_lane_name() {
        let lane = automation_lane("Drive : Distortion", "");
        let (fills, texts, _) = painted(&lane);
        let (content, stops) = stops();
        assert_eq!(
            texts,
            [("Drive : Distortion".to_owned(), content.to_color32())]
        );
        // In the miniature (123.4 ..= 149, 25.6 tall), a value v's box runs from 149 − 1 − ⌊25.6·v⌋ down to 149: from
        // each point to the next, the last a pixel wide; each pass from its start. In the tint at 0.8.
        let ink = sample(&stops, 0.8).to_color32();
        let boxes = |x: f32| {
            [(0.0, 64.0, 148.0), (64.0, 96.0, 123.0), (96.0, 97.0, 136.0)]
                .map(|(a, b, top)| (Rect::from_x_y_ranges(x + a..=x + b, top..=149.0), ink))
        };
        assert_eq!(fills[1..], [boxes(100.0), boxes(228.0)].concat());
        // Dimmed: the same area at the lowest stop.
        let dimmed = Look {
            dimmed: true,
            ..Look::default()
        };
        let (fills, ..) = painted_with(&lane, None, dimmed);
        assert!(fills[1..].iter().all(|f| f.1 == stops[0].to_color32()));
    }

    #[test]
    fn an_automation_clip_is_named_by_its_custom_label_then_its_auto_name_then_the_channel() {
        let name =
            |label: &str, custom: &str| painted(&automation_lane(label, custom)).1[0].0.clone();
        assert_eq!(name("Level", "Mine"), "Mine");
        assert_eq!(name("Level", ""), "Level");
        assert_eq!(name("", ""), synth().channels[1].name);
    }

    #[test]
    fn an_automation_loop_narrower_than_a_pixel_draws_nothing() {
        // A one-tick loop, a quarter point wide: a pass per tick would be many boxes a pixel.
        let mut lane = automation_lane("Level", "");
        lane.clips[0].loop_len_beats = 1.0 / BEAT;
        lane.clips[0].events.truncate(1);
        assert_eq!(painted(&lane).0.len(), 1);
        // Four ticks, a whole point: drawn.
        lane.clips[0].loop_len_beats = 4.0 / BEAT;
        assert!(painted(&lane).0.len() > 1);
    }

    #[test]
    fn a_notes_or_drum_lane_with_a_label_is_named_by_its_channel() {
        for kind in [LaneKind::Notes, LaneKind::Drums] {
            let mut lane = Lane::new(kind);
            lane.label = "Piano".into();
            lane.file.custom_label = "Mine".into();
            lane.add_clip(0, 4.0);
            assert_eq!(painted(&lane).1[0].0, synth().channels[1].name, "{kind:?}");
        }
    }

    /// The boxes [`automation`] fills of `clip` in red as (left, right, top), `view` the painter's clip rect, at 32
    /// points a beat from x = 100, values over y 0 ..= 49 (a box from 48 − ⌊48·v⌋), ppp 1.
    fn automation_boxes(clip: &Clip, view: Rect) -> Vec<(f32, f32, f32)> {
        let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
        let area = Rect::from_x_y_ranges(0.0..=10_000.0, 1.0..=49.0);
        let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let painter = ui.painter().with_clip_rect(view);
                automation(&painter, clip, area, AXIS, Color32::RED, &m);
            });
        });
        (out.shapes.into_iter())
            .filter_map(|c| match c.shape {
                egui::Shape::Rect(r) if r.fill == Color32::RED => {
                    Some((r.rect.left(), r.rect.right(), r.rect.top()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_offset_clip_draws_pass_0_from_before_its_start() {
        // Offset 1 beat: pass 0's content 0 sits a beat before the clip (x 68); the clip window cuts it.
        let mut clip = automation_lane("Level", "").clips[0].clone();
        clip.offset_beats = 1.0;
        let view = Rect::from_x_y_ranges(100.0..=356.0, 0.0..=49.0);
        let lefts: Vec<f32> = automation_boxes(&clip, view).iter().map(|b| b.0).collect();
        assert_eq!(lefts, [68.0, 132.0, 164.0, 196.0, 260.0, 292.0, 324.0]);
    }

    #[test]
    fn before_a_passs_first_point_the_previous_point_holds_from_the_pass_start() {
        // First point on beat 2: each pass starts at the last point's value (½) up to it. A view from beat 4.5 starts
        // in pass 1, whose start (beat 4) holds ½ to beat 6, then 1 from there.
        let mut clip = automation_lane("Level", "").clips[0].clone();
        clip.visible_len_beats = 16.0;
        clip.events.remove(0);
        let view = Rect::from_x_y_ranges(100.0 + 4.5 * 32.0..=100.0 + 6.0 * 32.0, 0.0..=49.0);
        assert_eq!(
            automation_boxes(&clip, view),
            [(228.0, 292.0, 24.0), (292.0, 324.0, 0.0)]
        );
        // From the clip start: pass 0 too.
        let view = Rect::from_x_y_ranges(100.0..=228.0, 0.0..=49.0);
        let first = automation_boxes(&clip, view)[0];
        assert_eq!(first, (100.0, 164.0, 24.0));
    }

    #[test]
    fn the_narrow_loop_guard_counts_a_pixel_per_128_points() {
        // A 4-tick loop is a pixel: 128 points draw, 129 need 4·129/128 ticks.
        let view = Rect::from_x_y_ranges(100.0..=356.0, 0.0..=49.0);
        let mut clip = automation_lane("Level", "").clips[0].clone();
        clip.loop_len_beats = 4.0 / BEAT;
        for (n, drawn) in [(128, true), (129, false)] {
            clip.events = (0..n)
                .map(|i| Event::Automation {
                    pos: i % 4,
                    value: 0.5,
                    control: true,
                })
                .collect();
            let boxes = automation_boxes(&clip, view);
            assert_eq!(!boxes.is_empty(), drawn, "{n} points");
        }
    }

    #[test]
    fn generated_steps_are_points_of_the_stepped_area() {
        // The stored steps between control points (control false) step the area as the control points do.
        let mut clip = automation_lane("Level", "").clips[0].clone();
        clip.events.insert(
            1,
            Event::Automation {
                pos: 128,
                value: 0.5,
                control: false,
            },
        );
        let view = Rect::from_x_y_ranges(100.0..=228.0, 0.0..=49.0);
        assert_eq!(
            automation_boxes(&clip, view),
            [
                (100.0, 132.0, 48.0),
                (132.0, 164.0, 24.0),
                (164.0, 196.0, 0.0),
                (196.0, 197.0, 24.0)
            ]
        );
    }

    #[test]
    fn a_recording_clip_is_a_channel_colour_box_a_pixel_in_and_a_content_colour_box_3_px_in() {
        // (x0, y0 + 1) … (x1, y1 − 1), then (y0 + 3) … (y1 − 3), at ppp 1 and 2.
        let theme = theme_by_id(0);
        let song = synth();
        let color = channel_color(song.channels[1].hue, &theme);
        let (content, _) = stops();
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
            let cx = UiCtx {
                theme: &theme,
                m: &m,
            };
            // A take two beats long on the row: its block, and its clip box.
            let rect = Rect::from_x_y_ranges(100.0..=164.0, TRACK.y_range());
            let b = clip_box(rect, &m);
            let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
                let panel = egui::CentralPanel::default().frame(egui::Frame::NONE);
                panel.show(ctx, |ui| {
                    paint_take(ui.painter(), &cx, &song.channels[1], rect);
                });
            });
            let fills: Vec<(Rect, Color32)> = (out.shapes.into_iter())
                .filter_map(|c| match c.shape {
                    egui::Shape::Rect(r) if r.fill != Color32::TRANSPARENT => {
                        Some((r.rect, r.fill))
                    }
                    _ => None,
                })
                .collect();
            let px = 1.0 / ppp;
            let inset = |n: f32| b.shrink2(vec2(0.0, n * px));
            assert_eq!(
                fills,
                [
                    (inset(1.0), color.to_color32()),
                    (inset(3.0), content.to_color32())
                ],
                "ppp {ppp}"
            );
        }
    }

    // ---- the selection overlay ----

    /// Master (0), Drums (1) and an Audio Clip channel (2); rows 64 points apart from y 48 (the master's), so Y_r =
    /// 48 + 64 r; 32 points a beat from x = 100. The visible rows are the last two: the master's is scrolled out above.
    fn three_tracks() -> (Song, Vec<(usize, usize, Rect)>) {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::AudioClip);
        let row =
            |r: f32| Rect::from_x_y_ranges(0.0..=1000.0, 48.0 + 64.0 * r..=48.0 + 64.0 * r + 63.0);
        (song, vec![(1, 0, row(1.0)), (2, 0, row(2.0))])
    }

    #[test]
    fn the_selection_box_spans_its_clips_from_the_first_rows_top_to_the_last_rows_bottom() {
        // The master's clip at beats 2 … 4 (its row scrolled out: y 48 … 112 by the rows' pitch) and the audio clip at
        // beats 6 … 10, with nothing selected on the Drums row between: one box, x 164 … 420, y 48 … 240 (Y_3), no
        // insets.
        let (mut song, rows) = three_tracks();
        song.channels[0].lanes[0].add_clip(256, 2.0);
        song.channels[2].lanes[0].add_clip(768, 4.0);
        let r = |channel| ClipRef {
            channel,
            lane: 0,
            id: 1,
        };
        let b = selection_box(&song, &[r(0), r(2)], &rows, AXIS, 1.0, false);
        assert_eq!(
            b,
            Some(Rect::from_min_max(pos2(164.0, 48.0), pos2(420.0, 240.0)))
        );
        // One clip: its start to its end across its own row.
        let b = selection_box(&song, &[r(2)], &rows, AXIS, 1.0, false);
        assert_eq!(
            b,
            Some(Rect::from_min_max(pos2(292.0, 176.0), pos2(420.0, 240.0)))
        );
        assert_eq!(selection_box(&song, &[], &rows, AXIS, 1.0, false), None);
    }

    #[test]
    fn the_capsules_end_0_5_u_inside_and_start_0_15_u_outside_and_the_start_one_hides_when_they_would_touch()
     {
        // U = 32: a capsule is 15.44 × 47.12 (0.42 U + 2 px by 1.41 U + 2 px), centred on the box vertically.
        let m = m1();
        let b = Rect::from_min_max(pos2(164.0, 48.0), pos2(420.0, 240.0));
        let near =
            |a: Rect, b: Rect| (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3;
        let capsule = |x: f32| Rect::from_min_size(pos2(x, 144.0 - 23.56), vec2(15.44, 47.12));
        let (start, end) = (capsule(164.0 + 16.0 - 15.44), capsule(420.0 + 4.8));
        // The audio bar: 1 U × (0.56 U + 2 px), centred on the right edge, 0.12 U below the box.
        let bar = Rect::from_min_size(pos2(404.0, 243.84), vec2(32.0, 19.92));
        let parts = |b: Rect, scale| handle_rects(b, scale, &m);
        let got = parts(b, true);
        let want = [(Part::Start, start), (Part::End, end), (Part::Scale, bar)];
        assert_eq!(got.len(), 3);
        for ((p, r), (q, w)) in got.into_iter().zip(want) {
            assert!(p == q && near(r, w), "{p:?} {r:?} vs {w:?}");
        }
        // 0.35 U wide, the start capsule's right edge would meet the end one's left: it hides. Any wider, it shows.
        let narrow = |w: f32| parts(Rect::from_min_size(b.min, vec2(w, 192.0)), false);
        assert_eq!(
            narrow(11.2).iter().map(|h| h.0).collect::<Vec<_>>(),
            [Part::End]
        );
        assert_eq!(narrow(11.3).len(), 2);
    }

    #[test]
    fn only_an_audio_clip_that_does_not_loop_takes_the_third_handle() {
        // length ≤ loop − content offset (wrapped into the loop).
        let mut audio = Lane::new(LaneKind::Audio);
        let mut clip = audio.add_clip(0, 4.0).clone();
        let audio = Lane::new(LaneKind::Audio);
        assert!(scalable(&audio, &clip));
        (clip.visible_len_beats, clip.offset_beats) = (3.0, 1.0);
        assert!(scalable(&audio, &clip));
        clip.offset_beats = 1.5;
        assert!(!scalable(&audio, &clip));
        clip.offset_beats = 5.0;
        assert!(scalable(&audio, &clip));
        (clip.visible_len_beats, clip.offset_beats) = (4.5, 0.0);
        assert!(!scalable(&audio, &clip));
        clip.visible_len_beats = 4.0;
        assert!(!scalable(&Lane::new(LaneKind::Notes), &clip));
    }

    #[test]
    fn the_third_handle_stretches_the_sample_as_the_audio_editors_stretch_handle_does() {
        // 2 s at 44.1 kHz and 120 BPM: 4 beats. Scaled by 1.5: L = 6 beats, STR1 ratio trunc(44100·60·6/120)/88200 =
        // 1.5, SYNC on; the clip keeps the visible length its drag gave it.
        let (mut song, _) = three_tracks();
        song.tempo_bpm = 120.0;
        let clip = song.channels[2].lanes[0].add_clip(0, 4.0);
        clip.file.sample = Some(Box::default());
        clip.visible_len_beats = 6.0;
        let r = ClipRef {
            channel: 2,
            lane: 0,
            id: 1,
        };
        assert!(stretch_clip(&mut song, r, (88_200, 44_100), 1.5));
        let clip = &song.channels[2].lanes[0].clips[0];
        let slot = clip.file.sample.as_deref().unwrap();
        assert_eq!((slot.file.stretch_mode, slot.file.stretch_ratio), (1, 1.5));
        assert_eq!(
            (clip.file.source_len_beats, clip.visible_len_beats),
            (6.0, 6.0)
        );
        // Again by 2: from the stretched 6 beats to 12.
        assert!(stretch_clip(&mut song, r, (88_200, 44_100), 2.0));
        let slot = song.channels[2].lanes[0].clips[0]
            .file
            .sample
            .as_deref()
            .unwrap();
        assert_eq!(slot.file.stretch_ratio, 3.0);
    }

    #[test]
    fn the_held_square_grows_to_3_u_at_300_ms() {
        // Side = 3 U·clamp(4.4333·t − 0.33, 0, 1).
        let u = 32.0;
        assert_eq!(hold_side(u, 0.0), 0.0);
        assert_eq!(hold_side(u, 0.074), 0.0);
        assert!(hold_side(u, 0.076) > 0.0);
        assert!((hold_side(u, 0.15) - 96.0 * (0.15 * 4.4333 - 0.33) as f32).abs() < 1e-3);
        assert!((hold_side(u, 0.3) - 96.0).abs() < 0.01);
        assert_eq!(hold_side(u, 1.0), 96.0);
        assert_eq!(hold_square(pos2(10.0, 10.0), u, 0.0), None);
        assert_eq!(
            hold_square(pos2(100.0, 100.0), u, 1.0),
            Some(Rect::from_center_size(pos2(100.0, 100.0), vec2(96.0, 96.0)))
        );
    }

    #[test]
    fn a_marquee_is_a_3_u_square_on_the_finger_selecting_its_point_until_dragged_then_the_rect() {
        let grid = Rect::from_min_max(pos2(50.0, 50.0), pos2(500.0, 300.0));
        let mut m = Marquee::at(pos2(100.0, 100.0), false);
        // U = 32: the calm zone is 16 points.
        m.follow(Some(pos2(110.0, 105.0)), 32.0);
        assert!(!m.dragged);
        assert_eq!(
            m.drawn(32.0),
            Rect::from_center_size(pos2(110.0, 105.0), vec2(96.0, 96.0))
        );
        assert_eq!(
            m.region(grid),
            Rect::from_min_max(pos2(110.0, 105.0), pos2(110.0, 105.0))
        );
        // Beyond the calm zone: the rect from the touch-down, no snapping; clamped to the grid to select.
        m.follow(Some(pos2(20.0, 400.0)), 32.0);
        assert!(m.dragged);
        assert_eq!(
            m.drawn(32.0),
            Rect::from_min_max(pos2(20.0, 100.0), pos2(100.0, 400.0))
        );
        assert_eq!(
            m.region(grid),
            Rect::from_min_max(pos2(50.0, 100.0), pos2(100.0, 300.0))
        );
        // Back inside the calm zone it stays dragged; a lift's frame (no pointer) leaves it where it was.
        m.follow(Some(pos2(101.0, 101.0)), 32.0);
        m.follow(None, 32.0);
        assert!(m.dragged && m.to == pos2(101.0, 101.0));
    }

    #[test]
    fn a_shown_rows_circles_pop_in_from_nothing_past_full_size_and_settle() {
        // 1.0515·sin(1.885·(1 − a)), a ← 0.6·a per frame.
        assert_eq!(row_pop_scale(1.0), 0.0);
        let a = |frames: i32| pop_in(ROW_POP, f64::from(frames) / 60.0);
        assert!(row_pop_scale(a(2)) < 1.0);
        assert!(row_pop_scale(a(4)) > 1.0);
        assert!(
            (row_pop_scale(a(4)) - 1.0515 * (1.885 * (1.0 - 0.6f32.powi(4))).sin()).abs() < 1e-4
        );
        assert_eq!(a(10), 0.0, "0.6^10 < 0.01: settled");
        assert_eq!(row_pop_scale(a(10)), 1.0);
    }
}
