//! The piano roll. It edits one clip's notes in place in the song and reports [`PianoRollAction::SongEdited`]:
//! `final_edit` is false while a drag is still changing the notes, so the app can rebuild the audio once, when the
//! gesture ends. A tap's new note is a [`PianoRollAction::QuietEdit`] instead; a graph-lane touch that changed nothing,
//! Quantize, Humanize and Snap to scale are a [`PianoRollAction::Resync`].
//!
//! Layout inside the rect it is given: the ruler strip on top, with the snap button in its left corner (the close X is
//! the app's, shared by the bottom editors: [`crate::ui::split::close_button`]); a vertical mini keyboard on the left;
//! the note grid in the rest, over the graph lane (`Vel` / `Pan`, [`graph_lane`]) while its tab shows it. The x axis
//! is the clip's content (tick 0 = content start). The grid background: no beat bands; rows light by scale and
//! instrument range, lit by selection and sound, the shared adaptive beat marks, root lines, and black α 0.15 past the
//! loop end ([`paint_grid`], [`paint_notes`]). Note names are not drawn.
//!
//! Gestures on the grid, each decided from its touch-down by the shared law ([`gesture::is_long`],
//! [`gesture::left_calm`]): within the 0.5 U calm zone a release after ≤ 0.3 s is a tap and after > 0.3 s a long press,
//! which acts only at its release; leaving the zone first makes the touch a drag.
//! - Touch-down on a note (bound by [`note_at`]: its row widened 0.2 row each way) makes it the whole selection, or
//!   acts on the whole selection when it lands inside the showing selection's frame (the selected notes' bounds); the
//!   handles and the floating row hide meanwhile. The note (or the selection) drags at once; a handle drags its own
//!   way (below).
//! - Tap on a note: if a selection was showing, the tap only deselects; else the note becomes the selection with its
//!   handles and floating row, after a re-check on its exact row (a tap in the 0.2-row margin selects nothing). So a
//!   second tap deselects: a tap never deletes a note.
//! - Long press on a note: the touch-down's selection shows its handles and floating row; it never toggles.
//! - The selection frame: the selected notes' box (earliest start to latest end, the highest note's row top to the
//!   lowest one's row bottom, one note's own rect) pulses, `text` at α 0.15 + 0.05·cos(2π·p), p + 0.0125 a drawn frame
//!   ([`clips::paint_pulse`]), whenever a note is selected: a marquee's catch, a quiet selection and a drag's notes
//!   too. Its handles ([`clips::editor_handles`]), drawn while the selection shows and no touch holds its notes, grow
//!   while held and wobble at their release:
//!   - the end capsule, 0.15 U right of the box, vertically centred on it: every selected note's length changes by as
//!     much as the box's width ([`stretch_to`]);
//!   - with ≥ 2 notes, the ↔ bar under the box's bottom-right: the notes scale in time about the box's start A,
//!     start' = A + (start − A)·k, length' = length·k, k = new width / width ([`scale_notes`]).
//!
//!   A handle takes no grab offset: the box's end goes to the finger's tick, rounded to the snap grid, at least grid/8
//!   (1/128 beat without snap) after the box's start ([`box_end`]); only the box's end snaps. Each drag is one undo
//!   step.
//! - Empty grid: a tap dismisses a visible floating menu, else clears the selection and adds a note ([`Remembered`]); a long press is the
//!   marquee ([`clips::Marquee`]): after 300 ms, dragging beyond 0.5 U makes a selection rectangle.
//!   While held still, copied notes offer the `Paste` insert menu over the finger's row at the snapped beat, with the
//!   insert cursor across the keys the paste takes ([`insert::paste`]). A later tap pastes there; dragging or another
//!   press hides it.
//!   Out of the calm zone on either axis (each its own 0.5 U) the touch is the scroller's ([`scroller`]): it scrolls,
//!   never a tap or a marquee. Two fingers zoom; the wheel zooms or scrolls by zone (`Self::scroll`).
//! - The right button ([`gesture::Erase`]): a press on a note (its drawn box)
//!   deletes it, and dragging on deletes every note the pointer's path crosses, with no box, all one undo step; a right
//!   click on empty grid that deleted nothing is `Paste` (as above).
//! - A move's release is neither a tap nor a long press, so it brings up no floating row: the moved notes stay selected
//!   with their handles and floating row hidden, a *quiet* selection, until a tap on a note (which selects that note),
//!   a long press, a box, a paste or a key shows one again. Its frame and handles take no touch, and an
//!   empty-grid tap clears it and creates a note. A handle's release shows the handles again.
//! - Sound: touching or tapping a note is silent. A new note sounds at velocity 1 for 150 ms; while a selection one key
//!   tall is dragged, each key change stops the old key and plays the new one at velocity 0.7, until the release
//!   ([`PianoRollAction::Note`]).
//! - The snap button: a tap toggles snap, a long press or a right click opens the snap menu: the time modes under
//!   `Time`, beside them (a Honeydew addition, GitHub issue #32) the graph lane's value snap for its shown item under
//!   `Vel` or `Pan` ([`crate::ui::automation::value::snap_items`]): `Off` (the default), `1 %` … `25 %`, then Vel's
//!   `MIDI` velocities or Pan's `L/C/R`. Each item keeps its own, an app setting. While it is on, the grid's lines show
//!   in the lane in place of its guides, and a paint lands on it ([`Prop::landed`]); a lane drag shows its value
//!   beside the finger.
//!
//! A move or a handle's drag changes nothing until the pointer has left the calm zone, so a long press never nudges a
//! note onto the grid.
//!
//! A new note: key = the row's, snapped to the project scale (an identity while it is Chromatic); start = the tick
//! floored to the snap unit (the visible grid spacing in Auto, unrounded with snap off), ≥ 0;
//! length = max(L, unit/8, or 1/128 beat with snap off);
//! velocity V, pan P, slide S; not selected. (L, V, P, S) is the [`Remembered`] note.
//!
//! A roll edits its clip only: without one (no id, or an id that is gone) it edits nothing and makes no clip (the app
//! opens it on the playlist's current clip, `ui/views/mod.rs` [`crate::ui::views::Slots::follow`]). Every edit fits the
//! loop to its notes ([`Clip::fit_loop`]: the smallest bar·2ᵏ beats that holds every note's end, which also shrinks it;
//! positions must stay inside the loop), and the visible length is left as it is (a new clip's 0 shows the loop; a
//! sized clip keeps its length and the content loops inside it). With a non-zero clip offset the content still starts
//! playing at the offset: bars added play right after the old loop end, and the notes before the offset (which play
//! after the wrap) move later by the bars added.
//!
//! Notes with a key byte above 127 (possible in a file) are kept in the song but neither drawn nor edited.
//!
//! The note menu's items act on the selection (Transpose, Snap to Scale, Toggle Slide and Guess Scale: or on every note
//! when nothing is selected), each as one final edit:
//! - `Snap to Scale` moves each note to the nearest tone of the project scale (the song's key lock, which the keyboard
//!   settings edit), the lower of two equally near ones.
//! - `Guess Scale...` opens its results as a dialog box on the dialog base ([`crate::ui::dialog`]: the screen dimmed,
//!   the rows in `text` on the floating panel, no sweep), centred on the screen: every root and scale but Chromatic,
//!   ranked by fewest accidentals, then most root notes, then picker order; 5 rows, 12 after `Show more`. A row sets
//!   the project key and scale; `Disable` sets the scale to Chromatic. Without an exact fit, `Choose` lists the closest
//!   ones.
//! - `Toggle Slide` flips each note's slide flag. Slide notes have no look of their own: they are drawn as other notes.
//! - `Transpose by...` turns the popup into a one-cell offset display: a vertical drag anywhere picks the offset (half
//!   a U a semitone, up is up, kept to what keeps every note in the key range) and its release applies it; a tap,
//!   Escape or a click outside cancel.
//! - `Humanize` opens the Humanize panel ([`humanize`], issue #29: it used to have no dialog), its seed
//!   drawn per use from the std hasher's random keys and egui's clock (OS entropy natively; on the web the keys are
//!   fixed, so the clock varies it). While it is open the roll draws its preview in place of the notes, on the grid
//!   and in the graph lane, and hides the floating row; its `Apply` writes the preview, re-sorted, deselects and
//!   re-syncs.
//!
//! The ruler takes the shared gestures ([`RangeSelect`]) in content ticks on the snap grid (whole ticks with snap off):
//! a tap sets the song's playhead, and the range is the song's play loop, content tick 0 being song tick
//! [`region::clip_origin`]. The playhead is the transport's, playing or stopped, where it plays the clip
//! ([`region::playhead_tick`]: in the song, or over Preview's span while Preview plays it); elsewhere it rests on the
//! clip's start, and the playhead items are off. Its disc is the playhead knob ([`timeline::PlayheadKnob`], as in every
//! editor): a tap plays, and a drag seeks to the snapped content tick under the finger within [the view's left edge,
//! the loop's end], the line and the disc following it. The region items act on every note of the clip ([`region`]);
//! `Snap to Scale` and `Guess Scale...` on the notes that start in the range. A region edit is one final edit and drops
//! the note selection.
//!
//! A Honeydew addition (issue #26): on a take being recorded ([`RollOptions::recording`]; the app opens the take's clip
//! in a roll open on its channel) the roll shows the clip as it records ([`crate::app::note_record::live`]: each held
//! note from its note-on to the playhead, the clip grown as the take will leave it) and edits nothing. Its playhead
//! runs on from the clip's start, unfolded, as the take grows to it; with `Continuous scrolling during playback` on the
//! view keeps it a quarter in, and with it off the view turns a page whenever it leaves the view.

mod humanize;
pub(crate) mod notes;

use std::hash::{BuildHasher, RandomState};

use eframe::egui::{self, Align2, Color32, Id, Pos2, Rangef, Rect, Sense, Vec2, pos2, vec2};

use crate::params::DRUM_SLOT_PARAMS;
use crate::song::scales::{CHROMATIC, SCALE_MASKS, scale_to_stored};
use crate::song::{Clip, Event, LaneKind, PPQ, Song, TimeSig};
use crate::theme::{Rgba, channel_color};
use crate::ui::automation::geom;
use crate::ui::automation::value::{self, Pick, ValueGrid, ValueSnap};
use crate::ui::automation::{GRID_ALPHA, GRID_GAP, value_label};
use crate::ui::clips::{self, Part, TimeAxis};
use crate::ui::dialog;
use crate::ui::drum_grid::edit::{MARK_GAP, mark_spacing, marks};
use crate::ui::edit_keys::{self, EditKey};
use crate::ui::graph_lane::{self, GraphLane, Prop};
use crate::ui::insert;
use crate::ui::keyboard::octave_label;
use crate::ui::popup::{self, PopupItem};
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::scroller::{self, Axis, Scroller, X, Y, Zones};
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RangeRuler, RangeSelect, vline_px};
use crate::ui::widgets::{self, SnapTouch};
use crate::ui::{UiCtx, gesture, pinch_press, primary_down_live};

use notes::*;
pub use notes::{Snap, SnapMode};

const BEAT: f64 = PPQ as f64;

// Sizes in U, from the playlist's proportions.
/// The key column, as wide as the playlist's track headers.
const KEYS_W: f32 = 1.5;
/// Default beat width and key-row height, and their zoom ranges.
const BEAT_U: f32 = 2.0;
const BEAT_U_RANGE: [f32; 2] = [0.25, 8.0];
const ROW_U: f32 = 0.5;
const ROW_U_RANGE: [f32; 2] = [0.25, 2.0];
/// The vertical key column: a C row's channel-colour bar is 0.1 U wide (ceil to whole px), its label starts 0.08 U
/// after the bar at 0.95 × the row height (its cap height half the row), up to 12 pt; below 6 pt the row has no label.
const ROOT_BAR: f32 = 0.1;
const LABEL_GAP: f32 = 0.08;
const LABEL_SHARE: f32 = 0.95;
const LABEL_PT: [f32; 2] = [6.0, 12.0];

// Colour treatments of theme roles.
/// Ghost notes: their channel colour at this alpha.
pub(crate) const GHOST_ALPHA: f32 = 0.35;
/// Rows that sound or hold a selected note: the channel colour at this alpha.
const LIT_ALPHA: f32 = 0.1;
/// The graph lane: its guides' and its (and the grid's) past-the-end overlay's black alpha (0.2 and 0.15).
const GUIDE_ALPHA: u8 = 51;
const PAST_END_LANE_ALPHA: u8 = 38;
/// Room to scroll past the loop end or the last note, in bars.
const TAIL_BARS: f64 = 2.0;
/// `Transpose by...`: the vertical drag, in U, that makes a semitone (the default key row).
const TRANSPOSE_STEP: f32 = 0.5;
/// `Guess Scale...`: the result rows shown before and after `Show more` (12, so that the dialog fits the shortest
/// window, 16.9 U, without scrolling).
const GUESS_ROWS: [usize; 2] = [5, 12];
/// A new note's sound: its velocity and length (seconds); a dragged key's velocity.
const NEW_VELOCITY: f32 = 1.0;
const NEW_GATE_S: f64 = 0.15;
const DRAG_VELOCITY: f32 = 0.7;

/// What the user did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PianoRollAction {
    /// The clip's events changed. `final_edit` is false while a drag goes on; rebuild the audio (and record undo)
    /// on the final one.
    SongEdited { final_edit: bool },
    /// A tap's new note: a final edit that never re-syncs.
    QuietEdit,
    /// A completed edit that re-syncs whatever it changed, with an undo step only if it changed the song: a graph-lane
    /// touch that changed nothing (its own undo re-syncs), Quantize, Humanize and Snap to scale.
    Resync,
    /// A tap on the ruler: the playhead goes to this song tick (or a playing transport stops), as on every ruler.
    SetPlayhead(u64),
    /// A tap on the playhead knob: the transport plays.
    Play,
    /// The playhead knob's drop: seek to this song tick, playing or stopped, as the playlist's knob does.
    Seek(u64),
    /// Sound `key` on the roll's channel at `velocity` (0..=1), or stop it (`None`): a new note, a dragged key.
    Note { key: u8, velocity: Option<f32> },
}

/// The remembered note: what a tap's new note takes. Set whenever exactly one note becomes the selection (a touch-down,
/// tap or long press on it, a box round it, the commit of its move or handle drag): L = max(its length, the
/// minimum length then), its velocity and pan (at most 254) and slide. Graph-lane `Vel` / `Pan` paints set V / P,
/// `Toggle Slide` sets S; `Set length` does not set L.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Remembered {
    /// Ticks.
    pub len: f64,
    pub velocity: u8,
    pub pan: u8,
    pub slide: bool,
}

impl Default for Remembered {
    /// One beat, velocity 0.8 (204), pan 0.5 (127), no slide.
    fn default() -> Self {
        Self {
            len: BEAT,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        }
    }
}

/// What the piano roll reads from the app each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RollOptions {
    /// Setting "Enable ghost notes".
    pub ghost_notes: bool,
    /// Setting "Set middle C to C4" (the C labels).
    pub middle_c_is_c4: bool,
    /// The transport's song tick, playing or stopped (every editor's playhead); 0 without an output.
    pub playhead: u64,
    /// How far past `playhead` the playhead is drawn ([`crate::audio_link::AudioLink::playhead_lead`]): the line, the
    /// disc and follow playhead only, never an edit.
    pub lead: u64,
    /// The transport plays.
    pub playing: bool,
    /// REC is armed: the playhead disc shows icon 5 (record).
    pub armed: bool,
    /// Preview plays this roll's clip: the playhead maps through its span ([`region::playhead_tick`]).
    pub previewing: bool,
    /// The pointer is over an overlay: the wheel, zoom and pinch are not the roll's.
    pub covered: bool,
    /// Setting `Continuous scrolling during playback`: the view follows the playhead.
    pub continuous: bool,
    /// The keys sounding on the roll's channel from the keyboard, typed notes and MIDI, and the engine's keys at the
    /// channel's input (song playback, previews), bit k for key k: their rows are lit (grid background step 4), with
    /// the roll's own sounds.
    pub sounding: u128,
    /// The roll's clip is a take being recorded: it shows live and read-only (module docs; Honeydew's own).
    pub recording: bool,
}

/// The piano roll of one clip: which clip, the snap setting, and view and gesture state.
pub struct PianoRoll {
    pub channel: usize,
    /// Index of the notes lane in the channel.
    pub lane: usize,
    /// `None`, or an id that is gone: the roll edits nothing (no note makes a clip).
    pub clip_id: Option<u32>,
    pub snap: Snap,
    /// Zoom: a beat's width and a key row's height, in U.
    beat_u: f32,
    row_u: f32,
    /// Points from tick 0 and from the top of key 127's row to the grid's top-left; set on the first frame.
    scroll: Option<Vec2>,
    /// The scroll and zoom controller.
    scroller: Scroller,
    /// One flag per clip event.
    selected: Vec<bool>,
    /// The selection is quiet: kept, its handles and floating row hidden, as a move's release leaves it (module docs).
    quiet: bool,
    /// The clip's events as this roll last left them: anything else at the start of a frame is an outside change
    /// (undo, a load), which drops the selection and any gesture.
    seen: Vec<Event>,
    gesture: Gesture,
    menu: Option<Menu>,
    /// The time range on the ruler.
    pub(crate) region: RangeSelect,
    /// The graph lane ([`graph_lane`]): its item, size and tab; and its drag: the clip's events at the drag's start,
    /// and whether the drag has reported an edit (its end then closes it, even with the notes back as they were).
    pub(crate) graph: GraphLane,
    graph_drag: Option<(Vec<Event>, bool)>,
    /// The graph lane's value snap, `Vel`'s and `Pan`'s ([`graph_lane::ROLL_PROPS`]): app settings, off by default.
    pub value_snap: [ValueSnap; 2],
    /// The floating buttons over the selection.
    pub(crate) row: clips::FloatingRow,
    /// The selection box's pulse ([`clips::paint_pulse`]) and its handles' press animation.
    pulse: f32,
    press: clips::HandlePress,
    /// What a new note takes.
    pub(crate) remembered: Remembered,
    /// A new note's sound: its key and when it stops (egui time).
    gate: Option<(u8, f64)>,
    /// The key the key column plays while it is held.
    key_down: Option<u8>,
    /// The grid cursor (content tick, key): where `paste` and `open` act ([`Self::keys`]).
    cursor: (f64, u8),
    /// The `Paste` button shows, at this finger (module docs).
    paste: Option<Pos2>,
    /// What a touch [`Self::open`] ended left to report: the next frame's first actions.
    pending: Vec<PianoRollAction>,
    /// The playhead knob on the ruler.
    knob: PlayheadKnob,
    /// The clip is a take being recorded ([`RollOptions::recording`]): nothing edits it.
    recording: bool,
}

/// A touch on the grid (module docs). Outside a touch, the selection is the one showing.
#[derive(Default)]
enum Gesture {
    #[default]
    Idle,
    /// Down on empty grid, not yet a tap, a long press or a drag; `shown`: a selection was showing.
    Press {
        origin: Pos2,
        since: f64,
        shown: bool,
    },
    Pan,
    /// The marquee: its selection is live.
    Box(clips::Marquee),
    Edit(Edit),
    /// A right-button swipe deleting the notes it crosses ([`gesture::Erase`]).
    Erase(gesture::Erase),
}

/// A move or a handle drag in progress, recomputed from the events as they were at its start.
struct Edit {
    /// What it holds: the notes ([`Part::Body`], a move), the end capsule or the ↔ bar.
    part: Part,
    /// The grabbed note (in `orig`), whose start a move snaps.
    anchor: usize,
    origin: Pos2,
    /// When it touched down (egui time), whether a selection was showing then, and which.
    since: f64,
    shown: bool,
    prior: Vec<bool>,
    /// An empty gap inside the selection can still drag the group, but a tap adds a note if its menu was hidden.
    create_on_tap: bool,
    orig: Vec<Event>,
    orig_sel: Vec<bool>,
    orig_loop: f64,
    /// The notes that change.
    which: Vec<bool>,
    /// The pointer has left the calm zone; until then the notes stay as they were.
    armed: bool,
    /// An ongoing edit went out, so the release reports the final one, even back where it started.
    reported: bool,
    /// The key sounding for a move of a selection one key tall.
    sound: Option<u8>,
}

#[derive(Clone, Copy)]
enum Menu {
    Snap,
    /// The note menu, centred on the `More...` button.
    More(Pos2),
    /// The note menu in transpose mode, at the same place: the offset so far, and where this drag started.
    Transpose {
        at: Pos2,
        offset: i32,
        from: Option<Pos2>,
    },
    /// The scale-guess results, with `Show more` on, and `Choose` taken, for the notes in `region` (the ruler's
    /// range) or else the note menu's targets.
    Guess {
        more: bool,
        closest: bool,
        region: Option<TimeRange>,
    },
    /// The Humanize panel ([`humanize`]).
    Humanize(humanize::Panel),
}

/// The scale-guess dialog's items.
#[derive(Clone, Copy)]
enum GuessItem {
    /// A candidate, by its index in the ranking.
    Row(usize),
    ShowMore,
    Choose,
    Disable,
    Ok,
}

/// The note menu's items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoreOp {
    Legato,
    SnapToScale,
    GuessScale,
    Humanize,
    DoubleUp,
    DoubleDown,
    ToggleSlide,
    /// Set length to this many ticks.
    Length(u32),
    TransposeBy,
    SelectAll,
}

/// The floating buttons over a selection.
const FLOATING: [&str; 4] = ["Copy", "Delete", "Snap", "More..."];

impl PianoRoll {
    pub fn new(channel: usize, lane: usize, clip_id: Option<u32>) -> Self {
        Self {
            channel,
            lane,
            clip_id,
            snap: Snap::default(),
            beat_u: BEAT_U,
            row_u: ROW_U,
            scroll: None,
            scroller: Scroller::default(),
            selected: Vec::new(),
            quiet: false,
            seen: Vec::new(),
            gesture: Gesture::Idle,
            menu: None,
            region: RangeSelect::default(),
            graph: GraphLane::default(),
            graph_drag: None,
            value_snap: [ValueSnap::OFF; 2],
            row: clips::FloatingRow::default(),
            pulse: 0.0,
            press: clips::HandlePress::default(),
            remembered: Remembered::default(),
            gate: None,
            key_down: None,
            cursor: CURSOR,
            paste: None,
            pending: Vec::new(),
            knob: PlayheadKnob::default(),
            recording: false,
        }
    }

    /// The first notes lane of `channel` and its first clip, if any. A channel without a notes lane (drums, audio)
    /// gets a roll that shows an empty grid and edits nothing.
    pub fn for_channel(song: &Song, channel: usize) -> Self {
        let lanes = song.channels.get(channel).map_or(&[][..], |c| &c.lanes);
        let lane = lanes.iter().position(|l| l.kind == LaneKind::Notes);
        let clip_id = lane.and_then(|i| lanes[i].clips.first()).map(|c| c.id);
        Self::new(channel, lane.unwrap_or(0), clip_id)
    }

    /// Edits clip `clip_id` of lane `lane` from now on. Another clip keeps the snap and the zoom only: its selection,
    /// gesture and menu start afresh, and it scrolls to its own notes. A touch under way ends ([`Self::end_touch`]),
    /// reported with the next frame.
    pub fn open(&mut self, lane: usize, clip_id: u32) {
        if (self.lane, self.clip_id) != (lane, Some(clip_id)) {
            let pending = self.end_touch();
            *self = Self {
                pending,
                snap: self.snap,
                beat_u: self.beat_u,
                row_u: self.row_u,
                graph: self.graph,
                remembered: self.remembered,
                gate: self.gate,
                key_down: self.key_down,
                ..Self::new(self.channel, lane, Some(clip_id))
            };
        }
    }

    fn clip<'a>(&self, song: &'a Song) -> Option<&'a Clip> {
        let lane = song
            .channels
            .get(self.channel)?
            .lanes
            .get(self.lane)
            .filter(|l| l.kind == LaneKind::Notes)?;
        lane.clips.iter().find(|c| Some(c.id) == self.clip_id)
    }

    fn clip_mut<'a>(&self, song: &'a mut Song) -> Option<&'a mut Clip> {
        if self.recording {
            return None;
        }
        let lane = song
            .channels
            .get_mut(self.channel)?
            .lanes
            .get_mut(self.lane)
            .filter(|l| l.kind == LaneKind::Notes)?;
        lane.clips.iter_mut().find(|c| Some(c.id) == self.clip_id)
    }

    /// The content tick the playhead shows at song tick `tick` ([`region::playhead_tick`]); on a take being recorded
    /// (Honeydew's own, module docs), from the clip's start on, unfolded: the take grows to it.
    fn playhead_at(&self, song: &Song, tick: u64, opts: &RollOptions) -> Option<f64> {
        match self.clip(song).filter(|_| self.recording) {
            Some(clip) => Some(tick as f64 - f64::from(clip.start_tick)),
            None => region::playhead_tick(self.clip(song), tick, opts.previewing),
        }
    }

    /// The ghost notes to draw over the content ticks `view`, `px` ticks a pixel ([`ghost_notes`]): none when the
    /// setting is off.
    // ponytail: rescanned every frame (every other notes clip's events in the view); cache it behind a song revision
    // counter if the song model ever gets one.
    fn ghosts(
        &self,
        song: &Song,
        opts: &RollOptions,
        view: std::ops::Range<f64>,
        px: f64,
    ) -> Vec<Ghost> {
        if !opts.ghost_notes {
            return Vec::new();
        }
        ghost_notes(
            song,
            self.channel,
            self.clip(song),
            song.time_sig.bar_beats(),
            view,
            px,
        )
    }

    /// The grid geometry for a grid whose top-left is `grid_min`, with `U` = `u` points.
    fn geom(&self, grid_min: Pos2, u: f32) -> Geom {
        Geom {
            origin: grid_min - self.scroll.unwrap_or_default(),
            beat_w: self.beat_u * u,
            row_h: self.row_u * u,
        }
    }

    /// Paints the piano roll over `rect` and handles its input. `song` is edited in place.
    ///
    /// Handle the returned actions in the same pass, before the next call: the song has already changed, and each
    /// action is reported once (a gesture's `SongEdited { final_edit: true }` is not repeated), so a dropped action
    /// loses its rebuild.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        cx: &UiCtx,
        song: &mut Song,
        opts: &RollOptions,
    ) -> Vec<PianoRollAction> {
        let m = cx.m;
        let u = m.u(1.0);
        let bpb = song.time_sig.bar_beats();
        let ruler = Rect::from_x_y_ranges(
            rect.x_range(),
            rect.top()..=rect.top() + timeline::ruler_h(m),
        );
        // Below the ruler: the grid, and under it the graph lane while it shows, sized by the editor's height below
        // its top window margin (0 here: the whole rect, ruler included).
        let (body, lane) = self.graph.split_below(rect, ruler.bottom(), m);
        // The key column, then a t gap: the grid and the time ruler start after it (the header column's rule).
        let keys = body.with_max_x(rect.left() + m.u(KEYS_W));
        let grid = body.with_min_x(keys.right() + m.t());
        let id = Id::new("piano_roll");
        self.recording = opts.recording;
        let mut actions = std::mem::take(&mut self.pending);
        // A new note's sound ends after its gate.
        let now = ui.input(|i| i.time);
        if let Some((key, off)) = self.gate {
            if now >= off {
                self.gate = None;
                actions.push(PianoRollAction::Note {
                    key,
                    velocity: None,
                });
            } else {
                ui.ctx().request_repaint_after_secs((off - now) as f32);
            }
        }

        // The selection follows the clip; an outside change (another clip, undo) drops it.
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        if events != self.seen.as_slice() || self.selected.len() != events.len() {
            self.seen = events.to_vec();
            self.selected = vec![false; events.len()];
            // A dragged key stops; the change (an undo) has dealt with the edit.
            if let Gesture::Edit(edit) = std::mem::take(&mut self.gesture) {
                self.stop_sound(&edit, &mut actions);
            }
            self.graph_drag = None;
        }
        if self.scroll.is_none() {
            self.scroll = Some(self.initial_scroll(song, grid, u));
        }

        self.row.covered = opts.covered;
        // The scroll and zoom controller first; the gestures then hand it empty-grid touches.
        let report = self.scroll(ui, id, (ruler, keys, grid), song, opts, u);
        // Touches on the key column play its keys and never scroll.
        let keys_touch = ui.interact(keys, keys_id(), Sense::click_and_drag());
        // A press held over a focus loss, or over a hide ([`keys_id`]), is up (see `primary_down_live`): its key stops,
        // and it plays no other.
        let live = primary_down_live(ui, keys_touch.id);
        let held = (keys_touch.is_pointer_button_down_on() && live)
            .then(|| ui.input(|i| i.pointer.interact_pos()))
            .flatten()
            .and_then(|p| self.geom(grid.min, u).key(p.y));
        if held != self.key_down {
            if let Some(key) = self.key_down {
                actions.push(PianoRollAction::Note {
                    key,
                    velocity: None,
                });
            }
            if let Some(key) = held {
                let velocity = Some(crate::app::KEY_VELOCITY);
                actions.push(PianoRollAction::Note { key, velocity });
            }
            self.key_down = held;
        }
        let response = ui.interact(grid, id.with("grid"), Sense::click_and_drag());
        crate::ui::focus::register(ui, id.with("grid"), grid, crate::ui::focus::Kind::Container);
        self.grid_input(ui, &response, grid, m, song, &mut actions);
        let g = self.geom(grid.min, u);

        // The grid and what lies on it: a take being recorded as it shows live.
        let live = (self.clip(song).filter(|_| self.recording))
            .map(|c| crate::app::note_record::live(c, opts.playhead, bpb));
        let shown = live.as_ref().or(self.clip(song));
        let events = shown.map_or(&[][..], |c| &c.events);
        // The Humanize panel's preview, in the notes' order (so the selection still fits it), drawn in their place.
        // ponytail: recomputed every frame the panel shows; keep it by (panel, events) if a huge clip makes that show.
        let preview = match self.menu {
            Some(Menu::Humanize(panel)) => Some(panel.preview(events, &self.selected)),
            _ => None,
        };
        let events = preview.as_deref().unwrap_or(events);
        let loop_ticks = shown.map_or(BEAT * f64::from(bpb), |c| c.loop_len_beats * BEAT);
        let color = channel_color(
            song.channels.get(self.channel).map_or(0.0, |c| c.hue),
            cx.theme,
        );
        // The t gap after the key column, under the playhead line.
        let gap = Rect::from_x_y_ranges(keys.right()..=grid.left(), body.y_range());
        (ui.painter()).rect_filled(gap, 0.0, cx.theme.colors.background.to_color32());
        let grid_painter = ui.painter_at(grid);
        let rows = self.rows(song, events, opts.sounding);
        paint_grid(&grid_painter, grid, &g, &rows, color, song.time_sig, cx);
        let view = g.tick(grid.left())..g.tick(grid.right());
        let px = g.tick(grid.left() + 1.0 / m.ppp()) - view.start;
        let end = g.x(loop_ticks).max(grid.left());
        let past = (end < grid.right()).then(|| grid.with_min_x(end));
        paint_notes(
            &grid_painter,
            &g,
            events,
            &self.selected,
            &self.ghosts(song, opts, view, px),
            past,
            color,
            cx,
        );
        // The playhead line's slot: over the notes, under their handles, the marquee and the floating row.
        let line_slot = grid_painter.add(egui::Shape::Noop);
        self.paint_selection(&grid_painter, &g, events, cx);
        #[cfg(any(target_arch = "wasm32", test))]
        crate::app::live::piano_roll(
            ui,
            cx,
            song,
            (grid, &g),
            (self.channel, self.lane, self.clip_id),
            match &self.gesture {
                Gesture::Edit(e) if e.reported => {
                    (&self.selected[..], Some((&e.orig[..], &e.which[..])))
                }
                // A swipe's deleted notes show the others as this person's selection, where they were, until its
                // release sends the change (and no ghosts meanwhile).
                Gesture::Erase(e) => (&[][..], Some((&e.orig[..], &e.gone[..]))),
                _ => (&self.selected[..], None),
            },
            opts.covered,
        );
        // The marquee: a square growing while a touch is held on empty grid, then the box.
        let now = ui.input(|i| i.time);
        let marquee = match self.gesture {
            Gesture::Press { origin, since, .. } => {
                ui.ctx().request_repaint();
                clips::hold_square(origin, u, now - since)
            }
            Gesture::Box(marquee) if marquee.dragged => Some(marquee.drawn(u)),
            _ => None,
        };
        if let Some(rect) = marquee {
            clips::paint_marquee(&grid_painter, rect, cx);
        }
        paint_keys(
            &ui.painter_at(keys),
            keys,
            &g,
            opts.middle_c_is_c4,
            color,
            cx,
        );
        if let Some(lane) = lane {
            let preview = preview.as_deref();
            self.graph_lane(ui, id, (lane, preview), &g, song, color, cx, &mut actions);
        } else {
            // The lane went mid-drag: the drag ends as its release would.
            let events = self.clip(song).map_or(&[][..], |c| &c.events);
            self.graph_drag_end(events, &mut actions);
        }

        // The ruler: a plain corner over the keys, bars over the grid, the range (the song's play loop, in the clip's
        // content ticks), and the playhead: the transport's where it plays the clip, else resting on its start.
        let (corner, bars) = ruler.split_left_right_at_x(grid.left());
        ui.painter()
            .rect_filled(corner, 0.0, cx.theme.colors.background.to_color32());
        let bars_painter = ui.painter_at(bars);
        let sig = song.time_sig;
        timeline::ruler(&bars_painter, bars, bars, g.origin.x, g.beat_w, sig, cx);
        let origin = region::clip_origin(self.clip(song));
        let playhead = self.playhead_at(song, opts.playhead, opts);
        let at = RangeRuler {
            strip: bars,
            axis: TimeAxis {
                origin: g.origin.x,
                beat_w: g.beat_w,
            },
            grid: self
                .snap
                .active(auto_grid(g.beat_w, u, song.time_sig), song.time_sig),
            sig,
            ink: color.to_color32(),
            range: TimeRange::in_clip(song.play_loop, origin),
            playhead,
        };
        if report.scrolled[X] || report.zoomed[X] {
            self.region.view_moved();
        }
        let out = self.region.show(ui, id.with("region"), &at, true, cx);
        timeline::scroll_thumb(&bars_painter, bars, at.axis, (0.0, loop_ticks), cx);
        // A ruler press waiting for its long press is the scroller's too, until its range starts.
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        if let Some(range) = out.range {
            song.play_loop = range.and_then(|r| r.song_loop(origin));
        }
        if let Some(tick) = out.playhead {
            let at = region::seek_tick(self.clip(song), tick, opts.previewing);
            actions.push(PianoRollAction::SetPlayhead(at));
        }
        if let Some((op, range)) = out.pick {
            self.region_op(op, range, playhead, song, &mut actions);
        }
        // Clipped at the key column's right edge: at beat 0 its left half shows in the t gap. While the knob is
        // dragged, the line and the disc follow the finger.
        let drawn = self.playhead_at(song, opts.playhead + opts.lead, opts);
        let x = (self.knob).line_x(ui, m, &at, loop_ticks - 1.0, g.x(drawn.unwrap_or(0.0)));
        let line_painter = ui.painter_at(body.with_min_x(keys.right()));
        line_painter.set(line_slot, timeline::playhead_line_shapes(grid, x, cx));

        // Buttons last, so they sit on top.
        // The ruler row's left cell, 1.5 U × (ruler h − 1 px).
        let snap_cell = Rect::from_min_size(
            corner.min,
            vec2(m.u(KEYS_W), corner.height() - 1.0 / m.ppp()),
        );
        let snap_center = snap_cell.center();
        self.snap_button(ui, id, snap_cell, color.to_color32(), cx);
        // The graph lane's tab, centred on the key column's right edge.
        let tab_x = rect.left() + m.u(KEYS_W);
        let below = (rect, ruler.bottom());
        let ink = color.to_color32();
        (self.graph).tab_below(ui, cx, id.with("graph tab"), below, tab_x, ink);
        if !matches!(self.menu, Some(Menu::Humanize(_))) {
            self.floating_buttons(ui, grid, &g, song, cx, &mut actions);
        }
        self.paste_button(ui, id, grid, &g, song, cx, &mut actions);
        // The playhead knob over them all: above the Snap button, which it overhangs at beat 0, and takes the touches
        // there while it shows, as the playlist's.
        self.knob.armed = opts.armed;
        let shown = timeline::knob_shown(opts.playing, self.row.showing());
        match (self.knob).show(ui, id.with("knob"), bars, x, shown, cx) {
            Some(KnobGesture::Tap) => actions.push(PianoRollAction::Play),
            Some(KnobGesture::Drop(x)) => {
                let tick = self.knob.drop_tick(ui, &at, loop_ticks - 1.0, x) as u32;
                let at = region::seek_tick(self.clip(song), tick, opts.previewing);
                actions.push(PianoRollAction::Seek(at));
            }
            None => {}
        }
        self.menus(ui, id, (snap_center, rect), song, &mut actions, cx);
        // After the menus, which take Escape first.
        self.keys(ui, grid, cx, song, &mut actions);
        // This frame's own edits are not outside changes.
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        if events != self.seen.as_slice() {
            self.seen = events.to_vec();
        }
        actions
    }

    /// Another person's change came into the song: the selection follows its notes ([`crate::app::live::follow`]), and
    /// the change is no outside change that drops it.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn follow(&mut self, song: &Song) {
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        self.selected = crate::app::live::follow(&self.seen, &self.selected, events, Event::pos);
        self.seen = events.to_vec();
    }

    /// The roll goes (the view left it, `app::left_view`) with a touch it will not see end ([`Self::end_touch`]); the
    /// key the key column holds and a new note's sound stop now.
    pub(crate) fn hide(&mut self) -> Vec<PianoRollAction> {
        let mut actions = self.end_touch();
        for key in [self.key_down.take(), self.gate.take().map(|g| g.0)] {
            actions.extend(key.map(|key| PianoRollAction::Note {
                key,
                velocity: None,
            }));
        }
        actions
    }

    /// The keys the roll sounds stop now, as at [`Self::hide`] (a hidden web page runs no frame, `app::on_hidden`): its
    /// key column's, a new note's and a dragged one's. A touch under way goes on, and ends with nothing to stop.
    pub(crate) fn silence(&mut self) -> Vec<u8> {
        let drag = match &mut self.gesture {
            Gesture::Edit(edit) => edit.sound.take(),
            _ => None,
        };
        let keys = [self.key_down.take(), self.gate.take().map(|g| g.0), drag];
        keys.into_iter().flatten().collect()
    }

    /// Ends a touch on the grid or the graph lane as its release would, before the roll stops seeing it: a dragged
    /// key stops, and a move or paint that reported an edit closes it with a final one, the notes where they are.
    /// With what an earlier end left unreported ([`Self::open`]).
    fn end_touch(&mut self) -> Vec<PianoRollAction> {
        self.paste = None;
        let mut actions = std::mem::take(&mut self.pending);
        // The clip's events as the roll last left them.
        let seen = std::mem::take(&mut self.seen);
        self.graph_drag_end(&seen, &mut actions);
        self.seen = seen;
        match std::mem::take(&mut self.gesture) {
            Gesture::Edit(edit) => {
                self.stop_sound(&edit, &mut actions);
                if edit.reported {
                    edited(&mut actions, true);
                }
            }
            Gesture::Erase(erase) if erase.erased() => edited(&mut actions, true),
            _ => {}
        }
        actions
    }

    /// The end of a graph-lane drag, if one is under way, with the clip's notes now `events`: one that changed nothing
    /// clears the selection and re-syncs, with no undo step (its own undo); else one that reported edits is a final
    /// edit.
    fn graph_drag_end(&mut self, events: &[Event], actions: &mut Vec<PianoRollAction>) {
        let Some((orig, reported)) = self.graph_drag.take() else {
            return;
        };
        if events == orig {
            self.selected.fill(false);
            actions.push(PianoRollAction::Resync);
        } else if reported {
            edited(actions, true);
        }
    }

    /// What the grid's key rows show ([`Rows`]): light where the key is in the project scale (Chromatic reads as C
    /// major) and in an instrument's key range on the channel (none without an instrument; a plugin instrument plays
    /// every key); lit where a note is selected, the roll sounds the key (a key-column touch, a new note's gate, a dragged key) or it is in
    /// `sounding` (the keyboard, typed notes, MIDI and the engine's keys on the channel, [`RollOptions::sounding`]).
    fn rows(&self, song: &Song, events: &[Event], sounding: u128) -> Rows {
        let (root, mask) = project_scale(song);
        let (from, mask) = if mask == SCALE_MASKS[CHROMATIC] {
            (0, SCALE_MASKS[0])
        } else {
            (root, mask)
        };
        let modules = song.channels.get(self.channel).map(|c| &c.rack.modules);
        let ranges: Vec<(i8, i8)> = (modules.into_iter().flatten())
            .filter_map(crate::ui::rack::key_range)
            .collect();
        let light = std::array::from_fn(|k| {
            let key = k as u8;
            in_scale(key, from, mask) && ranges.iter().any(|r| (r.0..=r.1).contains(&(key as i8)))
        });
        let mut lit = [false; 128];
        let selected = (events.iter().zip(&self.selected)).filter(|(_, s)| **s);
        let drag = match &self.gesture {
            Gesture::Edit(edit) => edit.sound,
            _ => None,
        };
        let own = [self.key_down, self.gate.map(|g| g.0), drag];
        let keys = selected.filter_map(|(e, _)| note(e).map(|n| n.2));
        for key in keys.chain(own.into_iter().flatten()) {
            if let Some(row) = lit.get_mut(usize::from(key)) {
                *row = true;
            }
        }
        for (k, row) in lit.iter_mut().enumerate() {
            *row |= sounding >> k & 1 == 1;
        }
        Rows { light, lit, root }
    }

    /// Centres the notes (or middle C) vertically, at tick 0.
    fn initial_scroll(&self, song: &Song, grid: Rect, u: f32) -> Vec2 {
        let keys: Vec<f32> = self
            .clip(song)
            .map_or(&[][..], |c| &c.events)
            .iter()
            .filter_map(note)
            .map(|(_, _, key)| f32::from(key))
            .collect();
        let key = if keys.is_empty() {
            60.0
        } else {
            keys.iter().sum::<f32>() / keys.len() as f32
        };
        let row_h = self.row_u * u;
        vec2(
            0.0,
            (f32::from(TOP_KEY) - key + 0.5) * row_h - grid.height() / 2.0,
        )
    }

    /// The roll's scroll and zoom controller: one finger scrolls both axes over the grid columns from the `ruler`'s top
    /// to the grid's bottom (not the `keys`, whose touches play keys, nor the graph lane), two zoom; the wheel zooms
    /// the rows over the key column and the time over the ruler, and scrolls over the grid, where a trackpad pinch
    /// zooms the time. With Settings `Continuous scrolling during playback` on, while playing a clip longer than the
    /// view, the view keeps the playhead (folded into the clip's loop, clip-relative) a quarter in, and one finger
    /// scrolls no time. The time scrolls over 0 … the loop end or the last note's bar, plus [`TAIL_BARS`]; the rows
    /// over the 128 keys. On a take being recorded (Honeydew's own, module docs) the playhead is unfolded, the time
    /// scrolls a view past it, and the view follows it with the setting on and turns a page with it off.
    fn scroll(
        &mut self,
        ui: &egui::Ui,
        id: Id,
        (ruler, keys, grid): (Rect, Rect, Rect),
        song: &Song,
        opts: &RollOptions,
        u: f32,
    ) -> scroller::Report {
        let ruler = ruler.with_min_x(grid.left());
        let zones = Zones {
            touch: [ruler.union(grid); 2],
            wheel_zoom: [ruler, keys],
            wheel_scroll: grid,
            pinch: grid,
        };
        let bpb = song.time_sig.bar_beats();
        let clip = self.clip(song);
        let end = clip.map_or(0.0, |c| {
            c.loop_len_beats.max(needed_loop_beats(&c.events, bpb))
        });
        let scroll = self.scroll.unwrap_or_default();
        let (beat_w, row_h) = (f64::from(self.beat_u * u), f64::from(self.row_u * u));
        let range = |r: [f32; 2]| [1.0 / f64::from(r[1] * u), 1.0 / f64::from(r[0] * u)];
        let mut axes = [
            Axis {
                pos: f64::from(scroll.x) / beat_w,
                zoom: 1.0 / beat_w,
                range: [0.0, end + TAIL_BARS * f64::from(bpb)],
                zoom_range: range(BEAT_U_RANGE),
                view: grid.x_range(),
            },
            Axis {
                pos: f64::from(scroll.y) / row_h,
                zoom: 1.0 / row_h,
                range: [0.0, f64::from(TOP_KEY) + 1.0],
                zoom_range: range(ROW_U_RANGE),
                view: grid.y_range(),
            },
        ];
        // A take being recorded: where its playhead is, in beats; the time scrolls a view past it.
        let take = (self.playhead_at(song, opts.playhead + opts.lead, opts))
            .filter(|_| self.recording)
            .map(|tick| tick / BEAT);
        if let Some(beat) = take {
            axes[X].range[1] = axes[X].range[1].max(beat + axes[X].span());
        }
        let was = axes;
        let loop_ticks = clip.map_or(0.0, |c| c.loop_len_beats * BEAT);
        let follow = match take {
            Some(beat) => opts.continuous.then_some(beat * BEAT),
            None => {
                (opts.continuous && opts.playing && loop_ticks / BEAT > axes[X].span()).then(|| {
                    geom::playhead(
                        (opts.playhead + opts.lead) as f64 - region::clip_origin(clip) as f64,
                        loop_ticks,
                    )
                })
            }
        };
        let options = scroller::Options {
            covered: opts.covered,
            follow: follow.is_some(),
        };
        let report = self
            .scroller
            .update(ui, id.with("scroll"), u, &zones, &mut axes, options);
        if let Some(tick) = follow {
            scroller::follow(&mut axes[X], tick / BEAT);
        } else if let Some(beat) = take
            && !(axes[X].pos..axes[X].pos + axes[X].span()).contains(&beat)
        {
            // Setting off: the page turns, the playhead at the left edge.
            axes[X].pos = beat.clamp(axes[X].range[0], axes[X].max());
        }
        let [x, y] = axes;
        if x.zoom != was[X].zoom {
            self.beat_u = (1.0 / x.zoom) as f32 / u;
        }
        if y.zoom != was[Y].zoom {
            self.row_u = (1.0 / y.zoom) as f32 / u;
        }
        if axes != was {
            self.scroll = Some(vec2((x.pos / x.zoom) as f32, (y.pos / y.zoom) as f32));
        }
        report
    }

    /// Touches and drags on the grid (module docs).
    fn grid_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        grid: Rect,
        m: &crate::metrics::Metrics,
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let u = m.u(1.0);
        let g = self.geom(grid.min, u);
        let sig = song.time_sig;
        let snap = self.snap.active(auto_grid(g.beat_w, u, sig), sig);
        let min_len = snap.map_or(1.0, |unit| unit / 8.0);
        let create_snap = self
            .snap
            .active(mark_spacing(g.beat_w, u, sig, MARK_GAP), sig);
        let create_min_len = create_snap.map_or(1.0, |unit| unit / 8.0);
        let (held, pressed, pos, now) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.primary_pressed(),
                // Unlike `latest_pos`, kept through a `PointerGone` in the release frame (a touch's lift).
                i.pointer.interact_pos(),
                i.time,
            )
        });
        // A press held over a focus loss is up (see `primary_down_live`): it ends the gesture as a release, and
        // starts or moves nothing while it lasts.
        let down = primary_down_live(ui, response.id);
        let stale = held && !down || ui.input(|i| !i.focused || crate::ui::focus_lost(i));
        let pos = pos.filter(|_| !stale);
        let moved = |o: Pos2| gesture::left_calm(o, pos, u);
        let multi = ui.input(|i| i.multi_touch().is_some());
        if multi || stale {
            self.paste = None;
        }
        // A second finger makes a pinch, which only zooms: it ends a one-finger gesture, putting back what it moved
        // and the selection its touch-down made, and starts none (not even a tap) until the lift.
        if multi && let Gesture::Edit(edit) = std::mem::replace(&mut self.gesture, Gesture::Pan) {
            self.stop_sound(&edit, actions);
            self.selected = edit.prior;
            if edit.reported
                && let Some(clip) = self.clip_mut(song)
            {
                (clip.events, clip.loop_len_beats) = (edit.orig, edit.orig_loop);
                edited(actions, true);
            }
        }
        // `seen` is the clip's events as of the start of this frame.
        let events = std::mem::take(&mut self.seen);
        self.quiet &= self.selected.contains(&true);

        // Touch-down: on a note or a handle the edit starts at once; on empty grid the touch waits to be decided.
        if pressed
            && down
            && !multi
            && response.is_pointer_button_down_on()
            && matches!(self.gesture, Gesture::Idle)
            && let Some(at) = pos
        {
            if crate::keymap::held(ui.ctx(), "editorSelect") {
                // The keymap's `editorSelect` (Ctrl/Cmd held): a marquee at once,
                // wherever the touch lands; the scroller lets it go, as for the selection modifier's on empty grid.
                self.gesture = Gesture::Box(clips::Marquee::at(at, true));
                self.scroller.yield_touch();
            } else {
                self.gesture = self.touch_down(&g, m, song, &events, at, now, min_len);
                // Empty grid is the scroller's; a note, a handle or the selection's frame is the edit's.
                if matches!(self.gesture, Gesture::Press { .. }) {
                    self.scroller.claim();
                }
            }
        }

        // A right press starts a swipe that deletes the notes it crosses (module docs).
        if matches!(self.gesture, Gesture::Idle)
            && let Some(erase) = gesture::Erase::start(ui, response, pos, &events, &self.selected)
        {
            self.paste = None;
            self.gesture = Gesture::Erase(erase);
        }
        self.seen = events;

        let up = !down;
        // A press on empty grid held past a tap is a long press: the marquee, which the finger then drags (decided
        // here, so that a move or a lift in the same frame counts); with the selection modifier it drags at once.
        let shift = ui.input(|i| i.modifiers.shift);
        if let Gesture::Press { origin, since, .. } = self.gesture
            && (gesture::is_long(now - since) || shift)
        {
            self.gesture = Gesture::Box(clips::Marquee::at(origin, shift));
            self.scroller.yield_touch();
        }
        match std::mem::take(&mut self.gesture) {
            Gesture::Press {
                origin,
                since,
                shown,
            } => {
                // Out of the scroller's calm zone on either axis it scrolls: no tap, no marquee.
                self.gesture = if self.scroller.left_calm().unwrap_or_else(|| moved(origin)) {
                    Gesture::Pan
                } else if up {
                    if !stale {
                        self.selected.fill(false);
                        if !shown {
                            self.add_note(
                                &g,
                                song,
                                origin,
                                create_snap,
                                create_min_len,
                                now,
                                actions,
                            );
                        }
                    }
                    Gesture::Idle
                } else {
                    Gesture::Press {
                        origin,
                        since,
                        shown,
                    }
                };
            }
            Gesture::Pan if down => self.gesture = Gesture::Pan,
            Gesture::Box(mut marquee) => {
                // Live: what the box touches is the selection, every frame.
                marquee.follow(pos, u);
                let events = self.clip(song).map_or(&[][..], |c| &c.events).to_vec();
                self.selected = in_box(&g, &events, marquee.region(grid));
                self.quiet = false;
                // A stationary hold offers Paste while notes are copied.
                self.paste =
                    (!stale && !multi && !marquee.dragged && !note_clipboard(ui.ctx()).is_empty())
                        .then_some(marquee.from);
                if !up {
                    self.gesture = Gesture::Box(marquee);
                }
            }
            Gesture::Edit(mut edit) => {
                edit.armed |= moved(edit.origin);
                if edit.armed
                    && let Some(p) = pos
                {
                    let delta = p - edit.origin;
                    self.drag(&mut edit, delta, &g, snap, song, actions);
                }
                if up {
                    if edit.create_on_tap
                        && !edit.armed
                        && !stale
                        && !gesture::is_long(now - edit.since)
                    {
                        self.selected.fill(false);
                        self.add_note(
                            &g,
                            song,
                            edit.origin,
                            create_snap,
                            create_min_len,
                            now,
                            actions,
                        );
                    } else {
                        self.release(edit, &g, now, min_len, song, actions);
                    }
                } else {
                    self.gesture = Gesture::Edit(edit);
                }
            }
            Gesture::Erase(mut erase) => {
                // What the swipe crossed goes at once, an ongoing edit; it shows in the grid, not under the keys.
                let left = erase.sweep(pos, |e| g.note_rect(e).map(|r| r.intersect(grid)));
                if let Some((events, sel)) = left
                    && let Some(clip) = self.clip_mut(song)
                {
                    clip.events = events;
                    clip.fit_loop(sig.bar_beats());
                    self.selected = sel;
                    edited(actions, false);
                }
                if gesture::Erase::held(ui) && !stale {
                    self.gesture = Gesture::Erase(erase);
                } else if erase.erased() {
                    // The release closes the swipe's one undo step.
                    edited(actions, true);
                } else if response.secondary_clicked()
                    && let Some(p) = pos
                    && let Some(Hit::Empty { .. }) = hit_test(&g, &self.seen, p)
                    && !note_clipboard(ui.ctx()).is_empty()
                {
                    // A right click on empty grid that deleted nothing is `Paste`.
                    self.paste = Some(p);
                }
            }
            Gesture::Pan | Gesture::Idle => {}
        }
    }

    /// The touch-down at `at` (module docs): the gesture it starts.
    #[allow(clippy::too_many_arguments)]
    fn touch_down(
        &mut self,
        g: &Geom,
        m: &crate::metrics::Metrics,
        song: &Song,
        events: &[Event],
        at: Pos2,
        now: f64,
        min_len: f64,
    ) -> Gesture {
        // A quiet selection is not showing: its frame and its (hidden) handles take no touch.
        let shown = self.selected.contains(&true) && !self.quiet;
        let frame = sel_box(g, events, &self.selected);
        let inside = shown && frame.is_some_and(|f| f.contains(at));
        let prior = self.selected.clone();
        let handle = (self.handles(frame, m).into_iter()).find(|h| h.2.contains(at));
        let hit = hit_test(g, events, at);
        if hit.is_some() || handle.is_some() {
            self.cursor = (g.tick(at.x).max(0.0), g.key(at.y).unwrap_or(self.cursor.1));
        }
        let first = self.selected.iter().position(|s| *s);
        let (part, anchor) = match (handle, hit) {
            (Some((part, ..)), _) => (part, first.unwrap_or(0)),
            (_, Some(Hit::Body(i))) if self.selected[i] => (Part::Body, i),
            (_, Some(Hit::Body(i))) if !inside => {
                self.selected = only(i, events.len());
                (Part::Body, i)
            }
            // Inside the showing selection's frame: it moves, the first selected note snapping.
            _ => match first {
                Some(first) if inside => (Part::Body, first),
                _ => {
                    return Gesture::Press {
                        origin: at,
                        since: now,
                        shown: self.row.showing(),
                    };
                }
            },
        };
        self.remember(events, min_len);
        let orig_loop = self.clip(song).map_or(0.0, |c| c.loop_len_beats);
        Gesture::Edit(Edit {
            part,
            anchor,
            origin: at,
            since: now,
            shown,
            prior,
            create_on_tap: part == Part::Body
                && matches!(hit, Some(Hit::Empty { .. }))
                && !self.row.showing(),
            orig: events.to_vec(),
            orig_sel: self.selected.clone(),
            orig_loop,
            which: self.selected.clone(),
            armed: false,
            reported: false,
            sound: None,
        })
    }

    /// A move or a handle's drag, `delta` points from its touch-down: the notes follow, and a selection one key tall
    /// sounds each key it reaches.
    fn drag(
        &mut self,
        edit: &mut Edit,
        delta: Vec2,
        g: &Geom,
        snap: Option<f64>,
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let bpb = song.time_sig.bar_beats();
        let (new, sel) = drag_edit(edit, delta, g, snap);
        let keys = || flagged(&edit.orig, &edit.which).map(|n| n.2);
        if edit.part == Part::Body
            && let (Some(low), Some(high)) = (keys().min(), keys().max())
            && low == high
        {
            let (down, up) = transpose_range(&edit.orig, &edit.which);
            let dk = (-(delta.y / g.row_h).round() as i32).max(down).min(up);
            let key = (i32::from(low) + dk) as u8;
            if key != edit.sound.unwrap_or(low) {
                self.stop_sound(edit, actions);
                actions.push(PianoRollAction::Note {
                    key,
                    velocity: Some(DRAG_VELOCITY),
                });
                edit.sound = Some(key);
            }
        }
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        if clip.events != new {
            // Back where it started: the notes and loop it began with, no edit to report.
            let moved = new != edit.orig;
            (clip.events, clip.loop_len_beats) = (new, edit.orig_loop);
            self.selected = sel;
            if moved {
                clip.fit_loop(bpb);
                edited(actions, false);
                edit.reported = true;
            }
        }
    }

    /// The release of a touch on a note, a handle or the selection's frame (module docs).
    fn release(
        &mut self,
        edit: Edit,
        g: &Geom,
        now: f64,
        min_len: f64,
        song: &Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        self.stop_sound(&edit, actions);
        // A move's release leaves its selection quiet; a tap, a long press or a handle's drag shows it (module docs).
        self.quiet = edit.armed && edit.part == Part::Body;
        if edit.armed {
            if edit.reported {
                edited(actions, true);
            }
        } else if !gesture::is_long(now - edit.since) {
            // A tap: a showing selection only clears; else the note alone (out of a quiet selection too), if the
            // touch-down lay on its exact row.
            let on = !edit.shown
                && edit.part == Part::Body
                && on_note(g, &edit.orig[edit.anchor], edit.origin);
            if on {
                self.selected = only(edit.anchor, self.selected.len());
            } else {
                self.selected.fill(false);
            }
        }
        let events = self.clip(song).map_or(&[][..], |c| &c.events).to_vec();
        self.remember(&events, min_len);
    }

    /// Stops the key a move sounds, if any.
    fn stop_sound(&self, edit: &Edit, actions: &mut Vec<PianoRollAction>) {
        if let Some(key) = edit.sound {
            actions.push(PianoRollAction::Note {
                key,
                velocity: None,
            });
        }
    }

    /// A tap on empty grid at `at` with nothing showing: a new note (module docs), sounded for [`NEW_GATE_S`].
    #[allow(clippy::too_many_arguments)]
    fn add_note(
        &mut self,
        g: &Geom,
        song: &mut Song,
        at: Pos2,
        snap: Option<f64>,
        min_len: f64,
        now: f64,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let tick = g.tick(at.x);
        let (Some(row_key), true) = (g.key(at.y), tick >= 0.0) else {
            return;
        };
        let (root, mask) = project_scale(song);
        let key = snap_key(row_key, root, mask);
        let start = snap_floor(tick, snap).max(0.0);
        let Remembered {
            len,
            velocity,
            pan,
            slide,
        } = self.remembered;
        let bpb = song.time_sig.bar_beats();
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let i = add_note(
            &mut clip.events,
            &mut self.selected,
            start,
            len.max(min_len),
            key,
        );
        if let Event::Note {
            velocity: v,
            pan: p,
            slide: s,
            ..
        } = &mut clip.events[i]
        {
            (*v, *p, *s) = (velocity, pan.min(254), slide);
        }
        clip.fit_loop(bpb);
        actions.push(PianoRollAction::QuietEdit);
        if let Some((key, _)) = self.gate.take() {
            actions.push(PianoRollAction::Note {
                key,
                velocity: None,
            });
        }
        actions.push(PianoRollAction::Note {
            key,
            velocity: Some(NEW_VELOCITY),
        });
        self.gate = Some((key, now + NEW_GATE_S));
    }

    /// The keymap's editing actions while the grid holds the keyboard (`ui/edit_keys.rs`): `copy` puts the selected
    /// notes on the [`note_clipboard`], `cut` and `delete` remove them, `paste` adds the clipboard at the grid cursor
    /// ([`Self::paste`]), `selectAll` selects every note, `close` deselects and `open` adds a note at the cursor, as a
    /// tap there would. Each edit is one final edit; each action hides the floating row (the zoom actions are the
    /// [`crate::ui::scroller::Scroller`]'s). No editing action during a touch.
    fn keys(
        &mut self,
        ui: &egui::Ui,
        grid: Rect,
        cx: &UiCtx,
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let ctx = ui.ctx();
        if !matches!(self.gesture, Gesture::Idle) {
            return;
        }
        let Some(key) = edit_keys::take(ctx) else {
            return;
        };
        self.row.hide();
        // A key shows the selection's handles, as before a move left it quiet.
        (self.paste, self.quiet) = (None, false);
        ctx.request_repaint();
        let u = cx.m.u(1.0);
        let g = self.geom(grid.min, u);
        let sig = song.time_sig;
        let snap = self.snap.active(auto_grid(g.beat_w, u, sig), sig);
        let min_len = snap.map_or(1.0, |unit| unit / 8.0);
        match key {
            EditKey::Close => self.selected.fill(false),
            EditKey::Open => {
                let snap = self
                    .snap
                    .active(mark_spacing(g.beat_w, u, sig, MARK_GAP), sig);
                let min_len = snap.map_or(1.0, |unit| unit / 8.0);
                let (tick, key) = self.cursor;
                // Only on an empty cell, as the step view's: a note of the key it would take over its start is one.
                let (root, mask) = project_scale(song);
                let (start, key_on) = (snap_floor(tick, snap).max(0.0), snap_key(key, root, mask));
                let over = |(pos, len, k): (f64, f64, u8)| {
                    k == key_on && pos <= start && start < pos + len
                };
                let notes = self.clip(song).map_or(&[][..], |c| &c.events);
                if !notes.iter().filter_map(note).any(over) {
                    let at = pos2(g.x(tick), g.y(key) + g.row_h / 2.0);
                    let now = ui.input(|i| i.time);
                    self.add_note(&g, song, at, snap, min_len, now, actions);
                }
            }
            EditKey::Paste => self.paste(ctx, song, snap, min_len, actions),
            EditKey::Copy | EditKey::Cut | EditKey::Delete | EditKey::SelectAll => {
                let Some(clip) = self.clip_mut(song) else {
                    return;
                };
                let sel = &mut self.selected;
                let chosen = (clip.events.iter().zip(sel.iter()))
                    .filter(|&(e, s)| *s && note(e).is_some())
                    .map(|(e, _)| *e);
                let chosen: Vec<Event> = chosen.collect();
                if key != EditKey::Delete && key != EditKey::SelectAll && !chosen.is_empty() {
                    set_note_clipboard(ctx, chosen);
                }
                match key {
                    EditKey::SelectAll => {
                        for (s, e) in sel.iter_mut().zip(&clip.events) {
                            *s = note(e).is_some();
                        }
                    }
                    EditKey::Cut | EditKey::Delete if remove_selected(&mut clip.events, sel) => {
                        clip.fit_loop(sig.bar_beats());
                        edited(actions, true);
                    }
                    _ => {}
                }
            }
        }
    }

    /// The `Paste` insert menu for the held position ([`insert::paste`]): over the tapped key's row at the snapped
    /// beat, its cursor across the keys the paste takes (as [`Self::paste`] moves them, kept inside the keys); a later
    /// tap pastes the note clipboard there.
    #[allow(clippy::too_many_arguments)]
    fn paste_button(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        grid: Rect,
        g: &Geom,
        song: &mut Song,
        cx: &UiCtx,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let Some((finger, key)) = self.paste.and_then(|p| Some((p, g.key(p.y)?))) else {
            self.paste = None;
            return;
        };
        let notes = note_clipboard(ui.ctx());
        let first =
            (notes.iter().filter_map(note)).min_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let Some((_, _, k0)) = first else {
            self.paste = None;
            return;
        };
        let sig = song.time_sig;
        let snap = self.snap.active(auto_grid(g.beat_w, cx.m.u(1.0), sig), sig);
        let tick = g.tick(finger.x).max(0.0);
        let keys = || notes.iter().filter_map(note).map(|n| i32::from(n.2));
        let (lo, hi) = (keys().min().unwrap_or(0), keys().max().unwrap_or(0));
        let dk = (i32::from(key) - i32::from(k0))
            .max(-lo)
            .min(i32::from(TOP_KEY) - hi);
        let (top, bottom) = (g.y((hi + dk) as u8), g.y((lo + dk) as u8) + g.row_h);
        let row = Rangef::new(g.y(key), g.y(key) + g.row_h);
        let at = (g.x(snap_round(tick, snap)), row, Rangef::new(top, bottom));
        let tap = insert::paste(ui, cx, id.with("paste"), grid, at);
        if tap.is_some() {
            self.paste = None;
        }
        if tap == Some(true) {
            self.cursor = (tick, key);
            let min_len = snap.map_or(1.0, |unit| unit / 8.0);
            self.paste(ui.ctx(), song, snap, min_len, actions);
        }
    }

    /// `paste`: the [`note_clipboard`]'s notes, their earliest (the lowest of those) on the grid cursor's cell, its
    /// tick rounded to the snap unit; the others keep their offsets in time and key, the group kept inside the keys.
    /// They become the selection (a single note is the remembered one). One final edit; nothing while the clipboard
    /// is empty.
    fn paste(
        &mut self,
        ctx: &egui::Context,
        song: &mut Song,
        snap: Option<f64>,
        min_len: f64,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let notes = note_clipboard(ctx);
        let first =
            (notes.iter().filter_map(note)).min_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        let Some((t0, _, k0)) = first else {
            return;
        };
        let bpb = song.time_sig.bar_beats();
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let mut which = vec![false; clip.events.len()];
        which.resize(clip.events.len() + notes.len(), true);
        clip.events.extend(notes);
        let (tick, key) = self.cursor;
        let dk = i32::from(key) - i32::from(k0);
        let mut events = shift_notes(&clip.events, &which, snap_round(tick, snap) - t0, dk);
        sort(&mut events, &mut which);
        clip.events = events;
        clip.fit_loop(bpb);
        let events = clip.events.clone();
        (self.selected, self.quiet) = (which, false);
        self.remember(&events, min_len);
        edited(actions, true);
    }

    /// Exactly one note of `events` is selected: it is the remembered note (with at least `min_len` ticks).
    fn remember(&mut self, events: &[Event], min_len: f64) {
        let mut chosen = (events.iter().zip(&self.selected)).filter(|(_, s)| **s);
        if let (Some((&e, _)), None) = (chosen.next(), chosen.next())
            && let Event::Note {
                len_beats,
                velocity,
                pan,
                slide,
                ..
            } = e
        {
            self.remembered = Remembered {
                len: (len_beats * BEAT).max(min_len),
                velocity,
                pan: pan.min(254),
                slide,
            };
        }
    }

    /// The selection box pulsing over the selected notes, a marquee's catch too, and its handles (module docs).
    fn paint_selection(&mut self, painter: &egui::Painter, g: &Geom, events: &[Event], cx: &UiCtx) {
        let sbox = sel_box(g, events, &self.selected);
        clips::paint_pulse(painter, cx, sbox, 0.0, &mut self.pulse);
        let now = painter.ctx().input(|i| i.time);
        let held = match &self.gesture {
            Gesture::Edit(e) if e.part != Part::Body => Some((e.part, e.since)),
            _ => None,
        };
        self.press.follow(held, now);
        for (part, rect, _) in self.handles(sbox, cx.m) {
            let growth = self.press.growth(painter.ctx(), part, now);
            clips::paint_handle(painter, cx, rect, (1.0, growth), part == Part::Scale);
        }
    }

    /// The handles on the selection box `sbox` with their touch zones: the end capsule, and the ↔ bar with ≥ 2 notes
    /// selected. None while the selection is quiet or a touch holds its notes.
    fn handles(&self, sbox: Option<Rect>, m: &crate::metrics::Metrics) -> Vec<(Part, Rect, Rect)> {
        let held = matches!(&self.gesture, Gesture::Edit(e) if e.part == Part::Body);
        let scale = self.selected.iter().filter(|s| **s).count() >= 2;
        match sbox {
            Some(b) if !self.quiet && !held => clips::editor_handles(b, true, scale, m),
            _ => Vec::new(),
        }
    }

    /// The graph lane in the strip `strip`: `Vel` / `Pan` in the key column, the value area under the grid, and its
    /// paint (see [`graph_lane::paint_notes`]), reported like a move; it draws `preview`'s notes when there is one.
    #[allow(clippy::too_many_arguments)]
    fn graph_lane(
        &mut self,
        ui: &mut egui::Ui,
        id: Id,
        (strip, preview): (Rect, Option<&[Event]>),
        g: &Geom,
        song: &mut Song,
        color: Rgba,
        cx: &UiCtx,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let list_w = m.u(KEYS_W);
        let labels = &graph_lane::ROLL_LABELS;
        let list = id.with("graph list");
        let content = graph_lane::frame(
            ui,
            cx,
            list,
            strip,
            list_w,
            labels,
            color.to_color32(),
            &mut self.graph.item,
        );
        // From the grid's x: the list column, then a t gap of the frame's `background` (as the grid's).
        let values = content.with_min_x(strip.left() + list_w + m.t());
        let prop = graph_lane::ROLL_PROPS[self.graph.item];
        let land = prop.land(self.value_snap[self.graph.item]);

        // The paint: at touch-down and at every move, the eligible notes under x take the value under y.
        let response = ui.interact(values, id.with("graph"), Sense::drag());
        // A press held over a focus loss is up (see `primary_down_live`): it ends the drag as a release.
        let live = primary_down_live(ui, response.id);
        // A second finger makes a pinch, as over the grid: it ends the drag, putting back what it painted, and paints
        // nothing until the lift.
        let pinch = pinch_press(ui, response.id);
        if pinch
            && let Some((orig, _)) = self.graph_drag.take()
            && let Some(clip) = self.clip_mut(song)
        {
            // What it painted (if anything) put back: a touch that changed nothing.
            clip.events = orig;
            actions.push(PianoRollAction::Resync);
        }
        // Every drag starts afresh, also after one that an undo cancelled (`Context::stop_dragging`) without an end.
        if response.drag_started() && live && !pinch {
            let events = self.clip(song).map_or(&[][..], |c| &c.events);
            self.graph_drag = Some((events.to_vec(), false));
        }
        // The finger and the value it paints, on the value grid while its snap is on.
        let touch = (response.interact_pointer_pos())
            .filter(|_| response.dragged() && live && self.graph_drag.is_some())
            .map(|p| (p, prop.landed(graph_lane::v_at(values, p.y), land)));
        if let Some((p, value)) = touch {
            let tick = g.tick(p.x);
            let painted = self.clip_mut(song).is_some_and(|clip| {
                graph_lane::paint_notes(&mut clip.events, &self.selected, prop, tick, value)
            });
            if painted {
                // V or P of the remembered note.
                let byte = u8::try_from(value).unwrap_or(u8::MAX);
                match prop {
                    Prop::Velocity => self.remembered.velocity = byte,
                    Prop::Pan => self.remembered.pan = byte.min(254),
                    Prop::Pitch | Prop::Repeat => {}
                }
                edited(actions, false);
                if let Some((_, reported)) = self.graph_drag.as_mut() {
                    *reported = true;
                }
            }
        }
        if response.drag_stopped() || !live {
            let events = self.clip(song).map_or(&[][..], |c| &c.events);
            self.graph_drag_end(events, actions);
        }

        // The value area: guides or the value grid, beat lines, the past-the-end overlay, then a stem and a cap per
        // note, and a drag's value label.
        let painter = ui.painter_at(values);
        let px = 1.0 / m.ppp();
        let (top, bottom, h) = (values.top(), values.bottom(), values.height());
        // A value's cap: Pan's from the centre, the others' from the bottom.
        let cap_y = |v: u16| {
            let p = prop.frac(v);
            m.round_px(match prop {
                Prop::Pan => top - px + h / 2.0 - (h - 2.0 * px) * (2.0 * p - 1.0) / 2.0,
                _ => bottom - h * p,
            })
        };
        painter.rect_filled(values, 0.0, colors.control_fill.to_color32());
        // The value grid while its snap is on (#32), as the automation editor's, in place of the guides: `text` lines
        // on the caps' rows, when at least 0.25 U apart.
        let lines = (land.grid())
            .filter(|&step| h * step as f32 / f32::from(prop.max()) >= m.u(GRID_GAP))
            .map(|_| land.lines(prop.max()));
        if let Some(lines) = lines {
            let ink = colors.text.to_color32().gamma_multiply(GRID_ALPHA);
            for y in lines.into_iter().map(cap_y) {
                let line = Rect::from_x_y_ranges(values.x_range(), y..=y + px);
                painter.rect_filled(line, 0.0, ink);
            }
        } else {
            // Ten 1 px guides, each the row ending at k·h/10: the 50 % one is the Pan stems' centre (lane.y − 1 +
            // h/2), and the 0 % one lies on the gap's last row, above the value area.
            let guides = ui.painter_at(values.with_min_y(top - px));
            let guide = Color32::from_black_alpha(GUIDE_ALPHA);
            for k in 0..10 {
                let y = m.round_px(top + k as f32 * h / 10.0);
                guides.rect_filled(
                    Rect::from_x_y_ranges(values.x_range(), y - px..=y),
                    0.0,
                    guide,
                );
            }
        }
        let bpb = song.time_sig.bar_beats();
        paint_marks(&painter, values, g, song.time_sig, cx);
        let loop_ticks = self
            .clip(song)
            .map_or(BEAT * f64::from(bpb), |c| c.loop_len_beats * BEAT);
        let end = g.x(loop_ticks).max(values.left());
        if end < values.right() {
            let past = Rect::from_x_y_ranges(end..=values.right(), values.y_range());
            painter.rect_filled(past, 0.0, Color32::from_black_alpha(PAST_END_LANE_ALPHA));
        }
        let tints = graph_lane::hit_tints(colors, color, prop);
        let events = preview.unwrap_or_else(|| self.clip(song).map_or(&[][..], |c| &c.events));
        for pass in [false, true] {
            for (e, _) in events
                .iter()
                .zip(&self.selected)
                .filter(|(_, s)| **s == pass)
            {
                let (Some((pos, len, _)), Some(v)) = (note(e), prop.get(e)) else {
                    continue;
                };
                let x0 = m.round_px(g.x(pos));
                let x1 = g.x(pos + len).max(x0 + px);
                let from = match prop {
                    Prop::Pan => top - px + h / 2.0,
                    _ => bottom,
                };
                let ink = graph_lane::hit_color(&tints, prop, v, pass);
                let y = cap_y(v);
                let stem = Rect::from_x_y_ranges(x0..=x0 + 4.0 * px, y.min(from)..=y.max(from));
                painter.rect_filled(stem, 0.0, ink);
                painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, y..=y + 2.0 * px), 0.0, ink);
            }
        }
        // The value under a dragging finger, on its cap's row ([`value_label`]).
        if let Some((p, v)) = touch {
            let at = pos2(p.x, cap_y(v) + px);
            let midi =
                land.grid().is_some() && self.value_snap[self.graph.item].grid == ValueGrid::Steps;
            let text = match prop {
                Prop::Velocity if midi => format!("MIDI {}", v >> 1),
                Prop::Velocity => format!("{} %", (100.0 * prop.frac(v)).round()),
                _ => DRUM_SLOT_PARAMS[1].display_text(prop.frac(v)),
            };
            value_label(&painter, cx, at, px, &text, &mut Vec::new());
        }
    }

    /// The snap button in `cell` ([`widgets::snap_button`]): a tap toggles snap, a long press or right click opens the
    /// snap menu.
    fn snap_button(&mut self, ui: &mut egui::Ui, id: Id, cell: Rect, ink: Color32, cx: &UiCtx) {
        match widgets::snap_button(ui, id.with("snap"), cell, (self.snap.on, ink), cx) {
            Some(SnapTouch::Menu) => self.menu = Some(Menu::Snap),
            Some(SnapTouch::Toggle) => self.snap.on = !self.snap.on,
            None => {}
        }
    }

    /// `Copy`, `Delete`, `Snap` and `More...` over the selection, in a [`clips::FloatingRow`].
    fn floating_buttons(
        &mut self,
        ui: &mut egui::Ui,
        grid: Rect,
        g: &Geom,
        song: &mut Song,
        cx: &UiCtx,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        let notes: Vec<(&Event, Rect)> = (events.iter().zip(&self.selected))
            .filter(|(_, s)| **s)
            .filter_map(|(e, _)| Some((e, g.note_rect(e)?)))
            .collect();
        let handles: Vec<Rect> = (self
            .handles(sel_box(g, events, &self.selected), cx.m)
            .iter())
        .map(|h| h.1)
        .filter(|h| h.intersects(grid))
        .collect();
        // A quiet selection has no row, which a release on it brings back neither.
        let rects = notes
            .iter()
            .map(|n| n.1)
            .filter(|_| !self.quiet && self.paste.is_none());
        let sel = rects.reduce(Rect::union).map(|rect| {
            let keys = notes.iter().filter_map(|(e, _)| note(e)).map(|n| n.2);
            clips::Selection {
                rect,
                one_row: keys.clone().min() == keys.max(),
                handles: &handles,
            }
        });
        let view = clips::GridView {
            grid,
            map: [g.origin.x, g.origin.y, g.beat_w, g.row_h],
        };
        let Some((i, center)) = self.row.show(ui, "note_button", sel, view, &FLOATING, cx) else {
            return;
        };
        let (bpb, sig) = (song.time_sig.bar_beats(), song.time_sig);
        let grid_ticks = self.snap.grid(auto_grid(g.beat_w, cx.m.u(1.0), sig), sig);
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let before = clip.events.clone();
        let sel = &mut self.selected;
        match FLOATING[i] {
            "Copy" => {
                let copied = clip
                    .events
                    .iter()
                    .zip(sel.iter())
                    .filter(|(e, s)| **s && note(e).is_some())
                    .map(|(e, _)| *e)
                    .collect();
                set_note_clipboard(ui.ctx(), copied);
                duplicate_selected(&mut clip.events, sel, grid_ticks);
            }
            "Delete" => {
                remove_selected(&mut clip.events, sel);
            }
            "Snap" => {
                quantize_selected(&mut clip.events, sel, grid_ticks);
            }
            _ => self.menu = Some(Menu::More(center)),
        }
        let changed = clip.events != before;
        if changed {
            clip.fit_loop(bpb);
            ui.ctx().request_repaint();
        }
        // Quantize re-syncs whatever it moved.
        if FLOATING[i] == "Snap" {
            actions.push(PianoRollAction::Resync);
        } else if changed {
            edited(actions, true);
        }
    }

    /// The open popup, if any: the snap menu, the note menu or one of its dialogs; the Humanize panel beside the roll
    /// `over`.
    fn menus(
        &mut self,
        ui: &egui::Ui,
        id: Id,
        (snap_center, over): (Pos2, Rect),
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
        cx: &UiCtx,
    ) {
        let ctx = ui.ctx();
        let bounds = ctx.content_rect();
        match self.menu {
            Some(Menu::Snap) => {
                // Beside the time modes, the value snap of the graph lane's item (#32): its percents, then Vel's MIDI
                // velocities or Pan's left, centre and right.
                let item = self.graph.item;
                let steps = ["MIDI", "L/C/R"][item];
                let grids = (ValueGrid::ALL.into_iter()).filter(|&g| g != ValueGrid::Rows);
                let grids = grids.map(|g| match g {
                    ValueGrid::Steps => (g, steps.to_owned()),
                    _ => (g, g.label()),
                });
                let vs = (self.value_snap[item], graph_lane::ROLL_LABELS[item]);
                let items = value::snap_items(self.snap.mode, vs, grids);
                let result =
                    popup::show(ctx, id.with("snap_menu"), snap_center, bounds, &items, cx);
                match result.selected {
                    Some(Pick::Time(mode)) => self.snap = Snap { on: true, mode },
                    Some(Pick::Value(grid)) => {
                        self.value_snap[item] = self.value_snap[item].picked(grid);
                    }
                    None => {}
                }
                if result.close {
                    self.menu = None;
                    ctx.request_repaint();
                }
            }
            Some(Menu::More(at)) => {
                let result = popup::show(ctx, id.with("note_menu"), at, bounds, &more_items(), cx);
                if result.close {
                    self.menu = None;
                    ctx.request_repaint();
                }
                if let Some(op) = result.selected {
                    let seed = RandomState::new().hash_one(ctx.input(|i| i.time).to_bits());
                    self.more(op, (at, seed), song, actions);
                }
            }
            Some(Menu::Transpose { at, offset, from }) => {
                self.transpose(ctx, id, (at, offset, from), song, actions, cx);
            }
            Some(Menu::Guess {
                more,
                closest,
                region,
            }) => {
                self.guess(ctx, id, (more, closest, region), song, actions, cx);
            }
            Some(Menu::Humanize(mut panel)) => {
                let hue = song.channels.get(self.channel).map_or(0.0, |c| c.hue);
                let ink = channel_color(hue, cx.theme).to_color32();
                let was = panel;
                let close = humanize::show(ctx, cx, id.with("humanize"), over, ink, &mut panel);
                self.menu = close.is_none().then_some(Menu::Humanize(panel));
                if close == Some(humanize::Close::Apply) {
                    self.humanize(panel, song, actions);
                }
                // The preview was drawn before the panel changed: draw it again.
                if close.is_some() || panel != was {
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    /// A region-menu item picked on `range` of the ruler, with the playhead at content tick `playhead` (module docs).
    fn region_op(
        &mut self,
        op: RegionOp,
        range: TimeRange,
        playhead: Option<f64>,
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        if !op.keeps_loop() {
            song.play_loop = None;
        }
        if op == RegionOp::GuessScale {
            self.menu = Some(Menu::Guess {
                more: false,
                closest: false,
                region: Some(range),
            });
            return;
        }
        let bpb = song.time_sig.bar_beats();
        let (root, mask) = project_scale(song);
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let before = clip.clone();
        if op == RegionOp::SnapToScale {
            snap_to_scale(&mut clip.events, &range.flags(&before.events), root, mask);
        } else {
            let p = playhead.map_or(0, |p| p as u32);
            region::edit_clip(clip, op, range, p, bpb);
        }
        let changed = *clip != before;
        if changed {
            self.selected = vec![false; clip.events.len()];
        }
        // Snap to scale re-syncs whatever it moved.
        if op == RegionOp::SnapToScale {
            actions.push(PianoRollAction::Resync);
        } else if changed {
            edited(actions, true);
        }
    }

    /// Applies a note-menu item to the selection, or opens its dialog in place of the menu at `at`; `seed` is the
    /// Humanize panel's first.
    fn more(
        &mut self,
        op: MoreOp,
        (at, seed): (Pos2, u64),
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let bpb = song.time_sig.bar_beats();
        let (root, mask) = project_scale(song);
        match op {
            MoreOp::TransposeBy => {
                self.menu = Some(Menu::Transpose {
                    at,
                    offset: 0,
                    from: None,
                });
                return;
            }
            MoreOp::GuessScale => {
                self.menu = Some(Menu::Guess {
                    more: false,
                    closest: false,
                    region: None,
                });
                return;
            }
            MoreOp::Humanize => {
                let panel = humanize::Panel::new(seed, self.selected.contains(&true));
                self.menu = Some(Menu::Humanize(panel));
                return;
            }
            _ => {}
        }
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let before = clip.events.clone();
        let which = targets(&clip.events, &self.selected);
        let sel = &mut self.selected;
        match op {
            MoreOp::Legato => legato_selected(&mut clip.events, sel),
            MoreOp::DoubleUp => double_selected(&mut clip.events, sel, 12),
            MoreOp::DoubleDown => double_selected(&mut clip.events, sel, -12),
            MoreOp::Length(ticks) => set_length_selected(&mut clip.events, sel, f64::from(ticks)),
            MoreOp::SelectAll => {
                for (s, e) in sel.iter_mut().zip(&clip.events) {
                    *s = note(e).is_some();
                }
                self.row.select_all();
            }
            MoreOp::SnapToScale => snap_to_scale(&mut clip.events, &which, root, mask),
            MoreOp::ToggleSlide => {
                toggle_slide(&mut clip.events, &which);
                // S of the remembered note: the first toggled note's new flag.
                let first =
                    (clip.events.iter().zip(&which)).find(|(e, on)| **on && note(e).is_some());
                if let Some((&Event::Note { slide, .. }, _)) = first {
                    self.remembered.slide = slide;
                }
            }
            // The dialogs opened above.
            MoreOp::GuessScale | MoreOp::TransposeBy | MoreOp::Humanize => {}
        }
        let changed = clip.events != before;
        if changed {
            clip.fit_loop(bpb);
        }
        // Snap to scale re-syncs whatever it changed (nothing, or only velocities, which the song diff alone takes for
        // a graph-lane edit).
        if op == MoreOp::SnapToScale {
            actions.push(PianoRollAction::Resync);
        } else if changed {
            edited(actions, true);
        }
    }

    /// The Humanize panel's `Apply`: its preview into the clip, re-sorted, the selection hidden, one final edit that
    /// re-syncs whatever it changed (nothing, or only velocities, which the song diff alone takes for a graph-lane
    /// edit).
    fn humanize(
        &mut self,
        panel: humanize::Panel,
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
    ) {
        let bpb = song.time_sig.bar_beats();
        let Some(clip) = self.clip_mut(song) else {
            return;
        };
        let before = clip.events.clone();
        clip.events = panel.preview(&clip.events, &self.selected);
        self.selected.fill(false);
        sort(&mut clip.events, &mut self.selected);
        if clip.events != before {
            clip.fit_loop(bpb);
        }
        actions.push(PianoRollAction::Resync);
    }

    /// `Transpose by...` (module docs), from the menu's `(at, offset, from)`: the popup shows the offset of the drag
    /// so far, and the drag's release applies it.
    fn transpose(
        &mut self,
        ctx: &egui::Context,
        id: Id,
        (at, offset, from): (Pos2, i32, Option<Pos2>),
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
        cx: &UiCtx,
    ) {
        let (pressed, released, pos) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
            )
        });
        let from = if pressed { pos } else { from };
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        let which = targets(events, &self.selected);
        let (down, up) = transpose_range(events, &which);
        let offset = match (from, pos) {
            (Some(a), Some(b)) => (((a.y - b.y) / cx.m.u(TRANSPOSE_STEP)).round() as i32)
                .max(down)
                .min(up),
            _ => offset,
        };
        let items = [PopupItem::new((), 0, 0, offset_label(offset))];
        let result = popup::show(
            ctx,
            id.with("transpose"),
            at,
            ctx.content_rect(),
            &items,
            cx,
        );
        self.menu = Some(Menu::Transpose { at, offset, from });
        if (released && from.is_some()) || result.close {
            if released
                && offset != 0
                && let Some(clip) = self.clip_mut(song)
            {
                clip.events = shift_notes(&clip.events, &which, 0.0, offset);
                edited(actions, true);
            }
            self.menu = None;
            ctx.request_repaint();
        }
    }

    /// `Guess Scale...`'s results (module docs), from the menu's `(more, closest, region)`: `Show more` on, `Choose`
    /// taken, the ruler's range it is for.
    fn guess(
        &mut self,
        ctx: &egui::Context,
        id: Id,
        (more, closest, region): (bool, bool, Option<TimeRange>),
        song: &mut Song,
        actions: &mut Vec<PianoRollAction>,
        cx: &UiCtx,
    ) {
        let events = self.clip(song).map_or(&[][..], |c| &c.events);
        let which = region.map_or_else(|| targets(events, &self.selected), |r| r.flags(events));
        let guesses = guess_scales(events, &which);
        let mut items: Vec<PopupItem<GuessItem>> = Vec::new();
        let line = |items: &mut Vec<_>, text: String| {
            items.push(PopupItem::header(0, items.len(), text));
        };
        let button = |items: &mut Vec<_>, item, label: &'static str, keep_open| {
            items.push(PopupItem {
                keep_open,
                ..PopupItem::new(item, 0, items.len(), label)
            });
        };
        match &guesses {
            None => {
                line(
                    &mut items,
                    "Please select at least three different notes.".into(),
                );
                button(&mut items, GuessItem::Ok, "OK", false);
            }
            Some(all) if all[0].accidentals > 0 && !closest => {
                line(&mut items, "No exact matches found.".into());
                line(
                    &mut items,
                    "Press 'Choose' to manually select from closest options or 'Disable' to deactivate scale \
                     locking."
                        .into(),
                );
                button(&mut items, GuessItem::Choose, "Choose", true);
                button(&mut items, GuessItem::Disable, "Disable", false);
            }
            Some(all) => {
                let name = song.channels.get(self.channel).map_or("", |c| &c.name);
                line(&mut items, format!("Analysis results for \"{name}\":"));
                // Exact fits lead the ranking, so they are its first rows.
                let fits = all
                    .iter()
                    .take_while(|g| closest || g.accidentals == 0)
                    .count();
                for (i, g) in all
                    .iter()
                    .take(fits.min(GUESS_ROWS[usize::from(more)]))
                    .enumerate()
                {
                    let row = items.len();
                    items.push(PopupItem::new(GuessItem::Row(i), 0, row, g.label(i + 1)));
                }
                if fits > GUESS_ROWS[0] {
                    let label = if more { "Show less" } else { "Show more" };
                    button(&mut items, GuessItem::ShowMore, label, true);
                }
                button(&mut items, GuessItem::Disable, "Disable", false);
            }
        }
        // A dialog box: the screen dimmed at α 0.6 under the floating panel round the rows, from the first frame, whose
        // rows are the popup's `text` items without its sweep or fill.
        let (bounds, id) = (ctx.content_rect(), id.with("guess"));
        let rect = popup::popup_rect(ctx, &items, bounds.center(), bounds, cx.m).intersect(bounds);
        dialog::paint_in_layers(ctx, id, (&cx.theme.colors, cx.m), bounds, rect);
        let result = popup::show_on_box(ctx, id, bounds.center(), bounds, &items, cx, true);
        let lock = (song.key_root, song.key_scale);
        match result.selected {
            Some(GuessItem::Row(i)) => {
                if let Some(g) = guesses.as_ref().and_then(|all| all.get(i)) {
                    song.key_root = i32::from(g.root);
                    song.set_key_scale(scale_to_stored(g.scale));
                }
            }
            Some(GuessItem::ShowMore) => {
                self.menu = Some(Menu::Guess {
                    more: !more,
                    closest,
                    region,
                });
            }
            Some(GuessItem::Choose) => {
                self.menu = Some(Menu::Guess {
                    more,
                    closest: true,
                    region,
                });
            }
            Some(GuessItem::Disable) => song.set_key_scale(scale_to_stored(CHROMATIC)),
            Some(GuessItem::Ok) | None => {}
        }
        if (song.key_root, song.key_scale) != lock {
            edited(actions, true);
        }
        if result.close {
            self.menu = None;
            ctx.request_repaint();
        }
    }
}

/// The grid cursor before any note is touched: content tick 0, key 60.
const CURSOR: (f64, u8) = (0.0, 60);

/// The key column's touch: one id for every channel's roll, so a press latched stale on one is stale on all (the app's
/// `left_view`).
pub(crate) fn keys_id() -> Id {
    Id::new("piano_roll").with("keys")
}

/// The piano roll's note clipboard (`copy`, `cut`): the notes last copied, at their positions, in egui's memory, so
/// that every channel's roll shares it.
pub(crate) fn note_clipboard(ctx: &egui::Context) -> Vec<Event> {
    ctx.data(|d| d.get_temp(Id::new("note_clipboard")))
        .unwrap_or_default()
}

fn set_note_clipboard(ctx: &egui::Context, notes: Vec<Event>) {
    ctx.data_mut(|d| d.insert_temp(Id::new("note_clipboard"), notes));
}

/// Only event `i` of `n` selected.
fn only(i: usize, n: usize) -> Vec<bool> {
    (0..n).map(|j| j == i).collect()
}

/// Records an edit; one per frame, and a final edit wins.
fn edited(actions: &mut Vec<PianoRollAction>, final_edit: bool) {
    for action in actions.iter_mut() {
        if let PianoRollAction::SongEdited { final_edit: f } = action {
            *f |= final_edit;
            return;
        }
    }
    actions.push(PianoRollAction::SongEdited { final_edit });
}

/// The events and selection of a move or handle drag dragged by `delta` points. A handle takes no grab offset: the
/// selection box's end goes to the finger's tick.
fn drag_edit(edit: &Edit, delta: Vec2, g: &Geom, snap: Option<f64>) -> (Vec<Event>, Vec<bool>) {
    let finger = g.tick(edit.origin.x + delta.x);
    let mut events = match edit.part {
        Part::Scale => scale_notes(&edit.orig, &edit.which, finger, snap),
        Part::End | Part::Start => stretch_to(&edit.orig, &edit.which, finger, snap),
        Part::Body => {
            let pos = note(&edit.orig[edit.anchor]).map_or(0.0, |n| n.0);
            let start = snap_round(pos + f64::from(delta.x / g.beat_w) * BEAT, snap);
            let dk = -(delta.y / g.row_h).round() as i32;
            shift_notes(&edit.orig, &edit.which, start - pos, dk)
        }
    };
    let mut sel = edit.orig_sel.clone();
    sort(&mut events, &mut sel);
    (events, sel)
}

/// The note menu: two columns, all enabled.
fn more_items() -> Vec<PopupItem<MoreOp>> {
    use MoreOp::*;
    let col0 = [
        (Legato, "Legato", true),
        (SnapToScale, "Snap to Scale", true),
        (GuessScale, "Guess Scale...", true),
        (Humanize, "Humanize", true),
        (DoubleUp, "Double Up", true),
        (DoubleDown, "Double Down", true),
        (ToggleSlide, "Toggle Slide", true),
    ];
    let col1 = [
        (Length(32), "Set length 1/16", true),
        (Length(64), "Set length 1/8", true),
        (Length(128), "Set length 1/4", true),
        (Length(256), "Set length 1/2", true),
        (Length(512), "Set length 1/1", true),
        (TransposeBy, "Transpose by...", true),
        (SelectAll, "Select All", true),
    ];
    [col0, col1]
        .into_iter()
        .enumerate()
        .flat_map(|(col, items)| {
            items
                .into_iter()
                .enumerate()
                .map(move |(row, (op, label, enabled))| PopupItem {
                    disabled: !enabled,
                    ..PopupItem::new(op, col, row, label)
                })
        })
        .collect()
}

/// `a` moved `t` of the way toward `b`.
fn mix(a: Rgba, b: Rgba, t: f32) -> Color32 {
    let ch = |x: f32, y: f32| x + (y - x) * t;
    Rgba::rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b)).to_color32()
}

/// What the grid's key rows show (grid background steps 2, 4 and 6).
pub(crate) struct Rows {
    /// Step 2: `moduleFill`, the key being in the project scale (Chromatic reads as C major) and in the key range of an
    /// instrument on the roll's channel.
    pub light: [bool; 128],
    /// Step 4: the channel colour α 0.1, the key sounding on the channel or holding a selected note.
    pub lit: [bool; 128],
    /// Step 6: the project key's pitch class, whose rows get the 2 px root line.
    pub root: u8,
}

/// Grid background steps 1–6: `controlFill` over the 128 key rows (from beat 0 when scrolled before it), the light
/// rows, every row's 1 px `background` α 0.3 top and bottom lines, the lit rows, the shared adaptive beat marks
/// ([`marks`]: `background`, 4 px bar, 2 px beat, 1 px finer, at least 0.6 U apart, w px from ⌊w/2⌋ px left of the
/// mark) and the root rows' 2 px `background` line at trunc(row bottom) − 2 px. Steps 7–9 are [`paint_notes`]'.
fn paint_grid(
    painter: &egui::Painter,
    grid: Rect,
    g: &Geom,
    rows: &Rows,
    color: Rgba,
    sig: TimeSig,
    cx: &UiCtx,
) {
    let (c, m) = (&cx.theme.colors, cx.m);
    let bg = c.background.to_color32();
    let px = 1.0 / m.ppp();
    let keys = Rangef::new(g.y(TOP_KEY), g.y(0) + g.row_h);
    let xs = Rangef::new(g.x(0.0).max(grid.left()), grid.right());
    if xs.min >= xs.max {
        return;
    }
    let area = Rect::from_x_y_ranges(xs, keys).intersect(grid);
    painter.rect_filled(area, 0.0, c.control_fill.to_color32());
    let shown = (g.key(grid.bottom()).unwrap_or(0)..=g.key(grid.top()).unwrap_or(TOP_KEY)).rev();
    let row = |key: u8| Rect::from_x_y_ranges(xs, g.y(key)..=g.y(key) + g.row_h);
    let light = c.module_fill.to_color32();
    for key in shown.clone().filter(|&k| rows.light[usize::from(k)]) {
        painter.rect_filled(row(key), 0.0, light);
    }
    let line = bg.gamma_multiply(0.3);
    for key in shown.clone() {
        let r = row(key);
        painter.rect_filled(r.with_max_y(r.top() + px), 0.0, line);
        painter.rect_filled(r.with_min_y(r.bottom() - px), 0.0, line);
    }
    let lit = color.to_color32().gamma_multiply(LIT_ALPHA);
    for key in shown.clone().filter(|&k| rows.lit[usize::from(k)]) {
        painter.rect_filled(row(key), 0.0, lit);
    }
    paint_marks(painter, area.with_min_x(grid.left()), g, sig, cx);
    for key in shown.filter(|&k| k % 12 == rows.root) {
        let y = m.floor_px(row(key).bottom()) - 2.0 * px;
        painter.rect_filled(
            Rect::from_x_y_ranges(grid.x_range(), y..=y + 2.0 * px),
            0.0,
            bg,
        );
    }
}

/// The shared adaptive beat marks over `area`'s height between its left and right edges, ticks as `g` maps them
/// ([`marks`]: `background`, 4 px bar, 2 px beat, 1 px finer, at least 0.6 U apart, w px from ⌊w/2⌋ px left of the
/// mark): the grid's (step 5) and the graph lane's.
fn paint_marks(painter: &egui::Painter, area: Rect, g: &Geom, sig: TimeSig, cx: &UiCtx) {
    let (m, bg) = (cx.m, cx.theme.colors.background.to_color32());
    let px = 1.0 / m.ppp();
    let (t0, t1) = (g.tick(area.left()), g.tick(area.right()));
    for (tick, w) in marks(t0, t1, g.beat_w, m.u(1.0), sig, MARK_GAP) {
        let x = m.round_px(g.x(tick)) - (w / 2.0).floor() * px;
        vline_px(painter, x, area.y_range(), w, bg, m);
    }
}

/// Grid background steps 7–9: the ghost notes, black α 0.15 `past` the clip's loop end, then the notes.
#[allow(clippy::too_many_arguments)]
fn paint_notes(
    painter: &egui::Painter,
    g: &Geom,
    events: &[Event],
    selected: &[bool],
    ghosts: &[Ghost],
    past: Option<Rect>,
    color: Rgba,
    cx: &UiCtx,
) {
    let (theme, m) = (cx.theme, cx.m);
    let px = 1.0 / m.ppp();
    // Flat cells with a 1-px gap right and below, so neighbours stay apart.
    let cell = |r: Rect| {
        Rect::from_min_max(
            r.min,
            pos2((r.right() - px).max(r.left() + px), r.bottom() - px),
        )
    };
    for ghost in ghosts {
        let top = g.y(ghost.key);
        let r = Rect::from_min_max(
            pos2(g.x(ghost.pos), top),
            pos2(g.x(ghost.pos + ghost.len), top + g.row_h),
        );
        let ghost_color = channel_color(ghost.hue, theme).to_color32();
        painter.rect_filled(cell(r), 0.0, ghost_color.gamma_multiply(GHOST_ALPHA));
    }
    if let Some(past) = past {
        painter.rect_filled(past, 0.0, Color32::from_black_alpha(PAST_END_LANE_ALPHA));
    }
    let (normal, pale) = (
        color.to_color32(),
        mix(color, theme.colors.text, graph_lane::SELECTED_TINT),
    );
    for (i, e) in events.iter().enumerate() {
        if let Some(r) = g.note_rect(e) {
            let fill = if selected.get(i) == Some(&true) {
                pale
            } else {
                normal
            };
            painter.rect_filled(cell(r), 0.0, fill);
        }
    }
}

/// The key column, vertical. A full-width row per key in `whiteKey` or `blackKey`, and a 1 px `background` α 0.3 line
/// where two white keys meet (under each C and F). Each C has a channel-colour bar down its left edge (the root marker
/// colour) and its octave label in `blackKeyDisabled` (as a root white key's label), left-aligned after the bar and
/// centred on the row.
fn paint_keys(
    painter: &egui::Painter,
    keys: Rect,
    g: &Geom,
    middle_c_is_c4: bool,
    color: Rgba,
    cx: &UiCtx,
) {
    let (c, m) = (&cx.theme.colors, cx.m);
    painter.rect_filled(keys, 0.0, c.background.to_color32());
    let px = 1.0 / m.ppp();
    let row = |key: u8| Rect::from_x_y_ranges(keys.x_range(), g.y(key)..=g.y(key) + g.row_h);
    let line = c.background.to_color32().gamma_multiply(0.3);
    let rows = g.key(keys.bottom()).unwrap_or(0)..=g.key(keys.top()).unwrap_or(TOP_KEY);
    for key in rows.clone() {
        let (r, black) = (row(key), matches!(key % 12, 1 | 3 | 6 | 8 | 10));
        let fill = if black { c.black_key } else { c.white_key };
        painter.rect_filled(r, 0.0, fill.to_color32());
        if matches!(key % 12, 0 | 5) {
            painter.rect_filled(r.with_min_y(r.bottom() - px), 0.0, line);
        }
    }
    let bar = (m.u_px() * ROOT_BAR).ceil() * px;
    let pts = LABEL_SHARE * g.row_h / m.u(1.0) * 16.0;
    let font = (pts >= LABEL_PT[0]).then(|| m.font_id_pt(pts.min(LABEL_PT[1])));
    for key in rows.filter(|k| k % 12 == 0) {
        let r = row(key);
        painter.rect_filled(r.with_max_x(r.left() + bar), 0.0, color.to_color32());
        if let Some(font) = &font {
            painter.text(
                pos2(r.left() + bar + m.u(LABEL_GAP), r.center().y),
                Align2::LEFT_CENTER,
                octave_label(key, middle_c_is_c4),
                font.clone(),
                c.black_key_disabled.to_color32(),
            );
        }
    }
}

#[cfg(test)]
mod tests;
