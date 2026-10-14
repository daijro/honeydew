//! The drum editor's step sequencer: the `SWING` button, one row per pad (pad name over sample name on a dark header
//! column), and the step grid. A long press (or secondary click) on a row header opens the two-column row menu.
//! `SWING` opens a popup with the swing slider; releasing it moves the clip's hits.
//! The rows are 2 U … 100/24 U apart (a pinch zooms them) and scroll under the strip, the headers with them, down
//! to the `+` add-drum button; each clip keeps its own zoom and scroll ([`ClipView`]). The graph lane under the rows,
//! while its knob shows it, shows and paints the selected row's hits ([`graph_lane`]). The knob's 2×2 U touch box wins
//! over the list's first item and a cut-off bottom row header where they overlap (see [`graph_lane`]).
//!
//! The drum mixer's fold (the host draws the mixer in the room):
//! - Tapping the current (selected) row's header toggles the drum mixer between collapsed and expanded; tapping any
//!   other row's header only makes that row current. A long press keeps the row menu; dragging does not resize.
//! - c ← c + 0.4·(t − c), snapping to t once |c − t| < 0.05 (t = 0 or 1); the header column is 3 U + c·12 U wide.
//!   It is stepped by input time, the 0.4 counted per 60 Hz frame, 0.6^(60·dt) left after dt seconds.
//! - Collapsed or animating, the meters are two thin vertical bars at the header's right edge; fully out they are
//!   horizontal at the bottom of the page area. Each bar is 2 hairlines thick, 1 apart, in the 5 hairlines the
//!   header's text leaves; L then R; the header's text colour on it, the channel colour under the page. At rest
//!   (no level feed) they are empty.
//! - The current row's header is filled with the channel colour (texts in `moduleFill`); on press it grows by up to
//!   10 px and wobbles on release. Other headers have no fill and channel-coloured texts. The growth is the arc
//!   slider's touch pulse applied to the fill, 10 px · f wider and taller about its centre; the texts stay.
//! - The keyboard/gamepad `Show mixer` / `Hide mixer` are not wired: the app has no shortcut table for them.
//!
//! `SWING` has the ruler row's top-left cell, 3 U × (1.5 U − 1 px), to itself: this view is the step-sequencer mode
//! (a step lane, `DrumGrid::Steps`), so the Snap button, which the free note-grid mode (a notes lane,
//! `DrumGrid::Notes`, another view) has in that cell, is never drawn here.
//!
//! Step selection (a touch is decided from its touch-down: a tap is released within 0.3 s and 0.5 U, a long press
//! later):
//! - Touch-down on a hit makes it the selection, unless it lands inside the showing selection's frame (the selected
//!   cells' bounds). A tap on a hit then deletes it when it alone is selected (the one just touched), else only
//!   deselects. A long press on a hit shows the touch-down's selection, never toggling. A move before 0.3 s is a drag,
//!   which scrolls the rows (step mode has no hit drag here).
//! - A tap on an empty step only deselects a showing selection, else toggles the step. A long press there starts a
//!   selection box at once; moving on without lifting stretches it, and the release selects the hits of every step
//!   it touches (lifted in place: nothing; `Paste` shows while the hit clipboard is set, below). A
//!   long press on a hit that then moves boxes too.
//! - A right click on a hit makes it the selection; on an empty step it is `Paste` (below).
//! - `Paste` (as the automation editor's): the floating `Copy` also stores the copied hits in the drum editors'
//!   clipboard ([`hit_clipboard`], shared with the free grid); while it is set, a long press lifted in place or a
//!   right click on an empty step shows the `Paste` button, which any other press hides. Its tap lands the
//!   clipboard's first step on the step under it, the others keeping their pads and distances in steps, in one final
//!   edit ([`StepAction::PasteSteps`]); the pasted hits are the selection.
//! - Selected hits take the selected tint (see below), and the floating buttons sit above them:
//!   `Copy` and `Delete`. This view is the step-sequencer mode (`DrumGrid::Steps`), so `Snap`, which only the free
//!   note-grid view shows, never does here. Each is one final edit ([`StepAction::CopySteps`],
//!   [`StepAction::DeleteSteps`]). A selected step whose hit goes (an undo, a row edit) leaves the selection.
//! - Every hit edit (a tap's toggle, Fill, Clear, Rotate, `Copy`, `Delete`) fits the loop to the hits
//!   ([`crate::song::Clip::fit_loop`]: the smallest bar·2ᵏ beats that holds them, which may shrink it), recomputed on
//!   every clip update, as the free grid and the piano roll do.
//!
//! The step grid ([`StepSeqView::paint_row`]): a cell per step from content beat 0 to the grid's right edge, past the
//! loop too (the grid has no end), 1 px short of the step and the row; in the loop the beats alternate `controlFill` /
//! `moduleFill`, past it every cell is `controlFill`, and nothing is a line. An active step sits on its own step's cell
//! ([`active_rect`], [`dashes`]). While the current row's header is held, that row is drawn last over a `background`
//! band, grown by the header's press. The playhead line ([`timeline::playhead_line`]) sits at the playhead folded into
//! the loop (s + ((pos − s) mod L), [`ruler::folded`]), playing or stopped, down to the last row.
//!
//! The time axis (step mode): a beat is U·d/(4k) wide, k in [0.1, 0.33]; a clip with no saved view opens at 3 U a beat
//! clamped (3.03 U at d = 4), from its earliest hit and lowest used row ([`new_view`]); posX scrolls over [0, 2L − the
//! beats in view]. The wheel zooms the time over the ruler, the rows over the headers and scrolls over the grid; a
//! two-finger pinch zooms both over the rows and the time over the ruler; a trackpad pinch over the grid zooms the
//! time; a drag on the grid scrolls both ways, one on the headers the rows. Follow playhead (the host's
//! [`StepSeqView::follow`]) keeps the folded playhead at a quarter of the view while it plays a loop longer than the
//! view. A tap anywhere on the grid's rows, past the loop too, adds a hit; the refit ([`crate::song::Clip::fit_loop`])
//! then grows the loop to the smallest b·2ᵏ that holds it, in the same action.
//!
//! The top strip holds the step grid's ruler right of the drum mixer's room (the editors' shared [`timeline::ruler`] in
//! song coordinates, so its badges are song bars, and the scroll indicator, [`ruler::indicator`]). It takes the shared
//! gestures ([`RangeSelect`]) in song ticks: a tap before the loop's end (s + L) sets the song's playhead
//! ([`StepAction::SetPlayhead`]) and past it does nothing, the range is the song's play loop
//! ([`StepAction::PlayLoop`]), and each region item is one final edit on every hit of the clip ([`StepAction::Region`],
//! the range in content ticks) that drops the step selection. The playhead knob sits on the ruler at the folded
//! playhead, a square 1.35 ruler heights wide with its disc 3 px in, the play icon or, while REC is armed
//! ([`StepSeqView::armed`]), the record icon; it shows only while stopped with no floating row, a tap plays
//! ([`StepSeqView::play`], which the app takes) and a drop seeks within [the view's left edge, s + L], on the step,
//! playing or stopped ([`StepAction::Seek`]), as the playlist's.
//!
//! Hit colour, pop-in and remembered values (both drum modes):
//! - The grid's hits and the graph lane's bars take the piano roll's note tint ([`hit_tints`], [`hit_color`]) with t
//!   from the graph-lane page ([`Prop::tint_t`], [`Prop::pull`]); selected hits the selected tint, over the channel
//!   colour halfway to `text`. The page is the lane's item whether the lane shows or not; a cell holding several hits
//!   shows the last one's colour (the one on top in the lane).
//! - A hit added by a tap pops in: its colour blends toward `text` by a, 1 on the frame it first shows, a ← 0.9·a per
//!   frame, 0 once under 0.01 ([`pop_in`]), counted per 60 Hz frame of input time, as the fold is; only a tap's hit
//!   pops (fills, copies and undos do not).
//! - [`Remembered`] (the view's [`StepSeqView::remembered`], for the free note-grid to share): whenever a gesture
//!   leaves exactly one hit in the selection (a right click or a long press's box on one hit, or a `Copy` of one hit),
//!   its length, velocity and pan are remembered; a graph-lane Velocity or Panning edit sets that value. A tap's new
//!   hit takes the remembered velocity and pan (defaults 255 and 127). A tap that deletes a hit remembers nothing (it
//!   never becomes the selection); step mode has no minimum length of its own (0), and the free note-grid applies its
//!   own when it uses the length; a pinch that puts a lane drag back leaves the remembered values as the drag set them.
//!
//! [`StepSeqView::show`] reads the song and returns a [`StepAction`]; [`StepSeqView::apply`] makes the edit on
//! the song's drum-lane events. Pad names come through [`DrumPads`], so the view does not depend on the kit
//! model. `Clone` and `Delete` change the kit as well, and `Set Name` / `Set MIDI Note` change only the kit: the
//! host applies those to its kit too. `Load Sample`, `Load Drumset`, `Save Drumset` and the `+` add-drum button
//! ([`add_drum`]) change nothing here: the host opens their picker or dialog.

pub(crate) mod edit;
pub(crate) mod ruler;

use std::collections::{BTreeSet, HashMap};

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align2, Area, Color32, Context, Id, Key, Order, Painter, Pos2, Rangef, Rect, Response,
    Sense, Ui, Vec2, pos2, vec2,
};

use crate::icons::{self, IconId};
use crate::metrics::{FontSize, Metrics};
use crate::song::{Channel, Clip, Event, Lane, LaneKind, Song, TimeSig};
use crate::theme::{DISABLED_ALPHA, Rgba, channel_color};
use crate::ui::buttons::outline_circle_button;
use crate::ui::clips::{
    FloatingRow, GridView, Marquee, Selection, TimeAxis, hold_square, paint_marquee,
};
use crate::ui::drum_grid::{self, ClipView};
use crate::ui::drums::{PAD_COUNT, header_names, meter_level};
use crate::ui::edit_keys::{self, EditKey};
use crate::ui::gesture;
use crate::ui::graph_lane::{self, GraphLane, Prop};
pub(crate) use crate::ui::graph_lane::{hit_color, hit_tints};
use crate::ui::popup::{self, PopupItem, PopupResult};
use crate::ui::region::{self, RegionOp, TimeRange};
use crate::ui::scroller::{self, Axis, Scroller, X, Y, Zones};
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RangeRuler, RangeSelect};
use crate::ui::widgets::{Control, ParamMenu, h_slider, pulse_growth};
use crate::ui::{UiCtx, pinch_press, primary_down_live};
use ruler::BEAT;

// Sizes in U.
/// The `SWING` strip on top: the playlist ruler's height.
pub(crate) const TOP_H: f32 = 1.5;
/// The row header column: the pad editor strip's 3 U header cell, which is the whole column while the drum mixer is
/// collapsed.
pub(crate) const HEADER_W: f32 = 3.0;
/// The drum mixer's page layer, fully out: with the header cell it makes the 15 U header column.
pub(crate) const MIXER_W: f32 = 12.0;
/// The fold's step per 60 Hz frame, and how near its target it snaps.
const FOLD_RATE: f32 = 0.4;
const FOLD_SNAP: f32 = 0.05;
/// The row pitch is the vertical zoom, clamped to 2 U … 100/24 U. A new clip asks for 1 U, so it opens at 2 U.
const PITCH_MIN: f32 = 2.0;
const PITCH_MAX: f32 = 100.0 / 24.0;
const NEW_PITCH: f32 = 1.0;
/// The scroll range is the rows plus this many rows of room, which holds the `+` add-drum button: `ADD_D` square,
/// `ADD_GAP` below the last row.
const ROOM_ROWS: f32 = 1.5;
const ADD_D: f32 = 2.0;
const ADD_GAP: f32 = 0.75;
/// The swing popup below `SWING`, and the inset of its slider (label `Swing` on top, the % value below).
const SWING_POPUP: [f32; 2] = [6.0, 3.0];
const SWING_INSET: f32 = 0.25;
/// The time zoom k in step mode: a beat is U·d/(4k) wide, so [(0.1/U)·4/d, (0.33/U)·4/d] beats per px (at d = 4 a
/// beat is 3.03 U … 10 U).
const STEP_K: [f32; 2] = [0.1, 0.33];
/// A clip with no saved view asks for this many U per beat; the zoom clamp then applies (3.03 U at d = 4).
const NEW_BEAT_U: f32 = 3.0;
/// The pop-in: step mode's a ← 0.9·a per frame; a is 0 once under [`POP_FLOOR`].
pub(crate) const STEP_POP: f32 = 0.9;
const POP_FLOOR: f32 = 0.01;
/// The floating buttons over the selected steps (step-sequencer mode).
const STEP_OPS: [&str; 2] = ["Copy", "Delete"];
/// The `Set Name` box.
const PROMPT_W: f32 = 8.0;
const PROMPT_ROW_H: f32 = 1.25;
/// Horizontal text padding, as in the popup.
const PROMPT_PAD: f32 = 0.5;
/// The graph lane's knob: its x from the view's left, in U.
pub(crate) const GRAPH_KNOB_X: f32 = 1.4;
/// The `Lock` button: the transport's round-button size.
const LOCK_D: f32 = 1.25;

const DELETE_PROMPT: &str = "Delete drum sample? (no undo)";
/// The `Set Name` prompt's title (`New sample name` is the audio clip's).
const RENAME_TITLE: &str = "Set Name";
const MIDI_PROMPT: &str = "Press a key on your MIDI controller to route it to the drum sample";
/// The `Paste` button's side, in U.
const PASTE: f32 = 1.5;

fn id() -> Id {
    Id::new("step_seq")
}

/// The drum editors' hit clipboard (as the automation editor's `Copy`): the hits the last floating `Copy` copied, at
/// their positions, in egui's memory, so that both drum modes share it, it outlives songs and views, and it is never
/// saved.
pub(crate) fn hit_clipboard(ctx: &Context) -> Vec<Event> {
    ctx.data(|d| d.get_temp(Id::new("drum_hit_clipboard")))
        .unwrap_or_default()
}

pub(crate) fn set_hit_clipboard(ctx: &Context, hits: Vec<Event>) {
    ctx.data_mut(|d| d.insert_temp(Id::new("drum_hit_clipboard"), hits));
}

/// The `Paste` button for a press at `finger` whose row band is `row` (a 1.5 U square labelled `Paste`, centred on the
/// finger's x and on the middle of the row under it; the automation editor's look, `menuFill` with the label in
/// `text`). `Some(true)` when tapped, `Some(false)` when a press lands anywhere else (it
/// hides), `None` meanwhile.
pub(crate) fn paste_button(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    finger: Pos2,
    row: Rangef,
) -> (Rect, Option<bool>) {
    let (c, m) = (&cx.theme.colors, cx.m);
    let rect = Rect::from_center_size(pos2(finger.x, row.center()), Vec2::splat(m.u(PASTE)));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    ui.painter()
        .rect_filled(rect, 0.0, c.menu_fill.to_color32());
    let font = m.font_id(FontSize::Pt12);
    let ink = c.text.to_color32();
    (ui.painter()).text(rect.center(), Align2::CENTER_CENTER, "Paste", font, ink);
    let elsewhere = ui.input(|i| i.pointer.any_pressed()) && !response.is_pointer_button_down_on();
    let tap = if response.clicked() {
        Some(true)
    } else {
        elsewhere.then_some(false)
    };
    (rect, tap)
}

/// What the step view reads from the drum kit, by pad index (= the drum event's `pad`, = the row).
pub trait DrumPads {
    /// How many pads the kit has. The grid shows at least this many rows, more if events use higher pads.
    fn count(&self) -> usize;
    /// The row header's lines of pad `pad` < [`Self::count`]: the pad name (`Kick`) over the sample name
    /// (`Synth Kick`), or a custom name over "" ([`crate::ui::drums::row_lines`]).
    fn names(&self, pad: usize) -> (&str, &str);
    /// The MIDI note routed to pad `pad` < [`Self::count`], if any.
    fn midi_note(&self, pad: usize) -> Option<u8>;
    /// Pad `pad`'s custom name, "" for none: what `Set Name` starts from.
    fn custom_name(&self, pad: usize) -> &str;
}

/// What the user did this frame. [`StepSeqView::apply`] makes the song edit.
#[derive(Clone, Debug, PartialEq)]
pub enum StepAction {
    /// A tap on a step: adds a hit one step long with the remembered velocity and pan at the step's start, moved by
    /// the lane's Swing on odd steps, or removes the hits in it.
    Toggle {
        pad: u8,
        step: usize,
    },
    /// The swing popup's slider was released (or reset) at a new `amount` in [0, 1]: the edited clip's hits move
    /// to `apply_drum_swing` of `from` (their positions when the popup opened, one per event), and the lane stores
    /// `amount`. Playback plays the moved hits; nothing applies swing again. `final_edit`: the edit is complete, so
    /// the host rebuilds the audio (drag frames before the release emit nothing).
    Swing {
        amount: f64,
        from: Vec<u32>,
        final_edit: bool,
    },
    /// `Fill every step` (1), `Fill each 2 step` (2), `Fill each 4 step` (4).
    Fill {
        pad: u8,
        every: usize,
    },
    Clear {
        pad: u8,
    },
    Rotate {
        pad: u8,
        right: bool,
    },
    /// Song: the row's hits are copied into a new row below, later rows move down. Kit: the host inserts a copy of
    /// the pad at `pad + 1`.
    Clone {
        pad: u8,
    },
    /// Confirmed. Song: the row's hits go, later rows move up. Kit: the host removes the pad.
    Delete {
        pad: u8,
    },
    /// Kit only: `Set Name`'s custom pad name for the pad, "" to remove it.
    Rename {
        pad: u8,
        name: String,
    },
    /// Kit only: the MIDI note routed to the pad; `None` is `Reset MIDI Note`.
    SetMidiNote {
        pad: u8,
        note: Option<u8>,
    },
    /// A graph-lane paint sample: the pad's hits in `step` take `value` for `prop` (only a selected one, while steps
    /// are selected). The drag goes on: the host records it as an ongoing edit.
    Graph {
        pad: u8,
        prop: Prop,
        step: usize,
        value: u16,
    },
    /// The graph-lane drag that sent [`Self::Graph`] ended, its hits changed: the host closes its edits as one undo
    /// step.
    GraphEnd,
    /// A graph-lane touch ended with the clip's hits as at its start, these (a pinch puts back what the drag painted):
    /// the clip's events go back to them. A touch that changes nothing undoes its own snapshot, and that undo
    /// re-syncs: the host re-syncs, with no undo step.
    GraphCancel(Vec<Event>),
    /// The floating `Copy` on the selected (pad, step) `cells`: their hits again right after them
    /// ([`edit::copy_steps`]). The copies become the selection.
    CopySteps {
        cells: Vec<(u8, usize)>,
    },
    /// The floating `Delete` on the selected (pad, step) `cells`: their hits go, and the selection with them.
    DeleteSteps {
        cells: Vec<(u8, usize)>,
    },
    /// The `Paste` button's tap: the clipboard's `hits` from `step` on ([`edit::paste_steps`]). They become the
    /// selection.
    PasteSteps {
        step: usize,
        hits: Vec<Event>,
    },
    /// A region-menu item on the ruler's `range` (content ticks), with the playhead at content tick `playhead`
    /// ([`region::edit_clip`]). It clears the song's play loop unless it is `Duplicate`.
    Region {
        op: RegionOp,
        range: TimeRange,
        playhead: u32,
    },
    /// The ruler's range changed: the song's play loop (song ticks) is now this. No edit.
    PlayLoop(Option<(u32, u32)>),
    /// A tap on the ruler: the playhead goes to this song tick (or a playing transport stops). The host's.
    SetPlayhead(u64),
    /// A tap on the free note-grid's playhead knob (the step grid's is [`StepSeqView::play`]): the transport plays. The
    /// host's.
    Play,
    /// A playhead knob's drop: seek to this song tick, playing or stopped, as the playlist's knob does. The host's.
    Seek(u64),
    /// The free note-grid's gestures ([`crate::ui::drum_grid`]): the edited clip's hits become `events`, with one
    /// selection flag each. `final_edit` is false while a drag goes on (the host keeps the undo step open). The step
    /// view makes no such edit.
    Hits {
        events: Vec<Event>,
        selected: Vec<bool>,
        final_edit: bool,
        /// A tap's new hit: no re-sync.
        quiet: bool,
    },
    /// The row menu's `Load Sample` on pad `pad`: the host opens the sample picker, whose pick gives the pad's slot the
    /// file. No edit here.
    LoadSample {
        pad: u8,
    },
    /// The `+` add-drum button: the host opens the sample picker, whose pick adds a pad. No edit here.
    AddDrum,
    /// The row menu's `Load Drumset` and `Save Drumset`: the host opens the drumset picker, or the `Save Drumset` dialog.
    /// No edit here.
    LoadDrumset,
    SaveDrumset,
}

/// The modal thing on top of the grid, if any.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Overlay {
    /// The row menu, opened at `at`.
    Menu {
        row: usize,
        at: Pos2,
    },
    ConfirmDelete(usize),
    Rename {
        row: usize,
        text: String,
    },
    /// Waiting for a MIDI key, see [`StepSeqView::midi_key`].
    LearnMidi(usize),
    /// The swing popup: the clip's positions when it opened, the slider's value, the last value applied, and the
    /// positions as the popup last left them. Positions or a lane swing other than those are an outside change (an
    /// undo), which re-seeds the popup from the clip as it is then.
    Swing {
        from: Vec<u32>,
        amount: f32,
        committed: f32,
        seen: Vec<u32>,
    },
}

/// `u` U of row pitch, clamped to the zoom range.
fn clamp_pitch(u: f32) -> f32 {
    u.clamp(PITCH_MIN, PITCH_MAX)
}

/// A clip's vertical zoom (the row pitch) and scroll, in U. The view keeps one per clip; the song does not.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowView {
    pub pitch: f32,
    pub scroll: f32,
}

impl Default for RowView {
    /// A new clip.
    fn default() -> Self {
        Self {
            pitch: clamp_pitch(NEW_PITCH),
            scroll: 0.0,
        }
    }
}

/// Which clip a [`RowView`] belongs to: (channel, lane, clip id), the id `None` while the lane has no clip.
type ViewKey = (usize, usize, Option<u32>);

/// State of the step view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepSeqView {
    /// The edited drum lane of the channel, and the clip in it, by id (`None`, or a clip that is gone: none, and the
    /// grid edits nothing).
    pub lane: usize,
    pub clip: Option<u32>,
    /// The selected row (pad).
    pub selected: usize,
    /// The transport's `Lock` button, shown in the drum view only.
    pub lock: bool,
    /// Room (points) between the row headers and the steps for the drum mixer's pages, which the host draws there (a
    /// strip is the header cell, then the page layer): c·12 U, set by [`Self::show`] from [`Self::fold`] every frame.
    pub mixer_w: f32,
    /// The pointer is over an overlay: the wheel, zoom and pinch are not the view's. The host sets it each frame.
    pub covered: bool,
    /// The transport's song tick, playing or stopped (every editor's playhead), 0 without an output; and whether it
    /// plays (the playhead's line shows). The host sets both each frame.
    pub playhead: u64,
    /// How far past `playhead` the playhead is drawn ([`crate::audio_link::AudioLink::playhead_lead`]): the line, the
    /// knob and follow playhead only, never an edit. The host sets it each frame.
    pub lead: u64,
    pub playing: bool,
    /// REC is armed: the playhead disc shows the record icon. The host sets it each frame.
    pub armed: bool,
    /// Preview plays the shown clip: the playhead maps through its span ([`region::playhead_tick`]). The host sets it
    /// each frame.
    pub previewing: bool,
    /// Per row, its pad's output peaks (L, R), linear, for the row's level meters. The host sets it each frame;
    /// a row past its end is silent.
    pub levels: Vec<[f32; 2]>,
    /// Each clip's zoom and scroll, rows and time (posX, posY, zoom x, zoom y). The host keeps them when it resets the
    /// view for another channel, and a new song drops them with the rest of the view. Undo keeps them: a clip that an
    /// undo removes and a later edit adds again gets its old id back, and so its old view, which is harmless (each
    /// frame clamps it to the new rows and loop). Kept in memory, as the free grid's and the piano roll's; the song's
    /// `ZOOM` stays as read, uninterpreted.
    pub(crate) views: HashMap<ViewKey, ClipView>,
    /// Setting `Continuous scrolling during playback` (follow playhead). The host sets it each frame.
    pub follow: bool,
    /// The playhead knob was tapped: the app starts playback and clears it.
    pub play: bool,
    /// The playhead knob on the ruler.
    knob: PlayheadKnob,
    /// The last frame's layout, `None` when it drew nothing.
    last: Option<Layout>,
    overlay: Option<Overlay>,
    /// The graph lane ([`graph_lane`]): its item, size and knob; and its drag: the clip's events at the drag's start,
    /// and whether it has sent an edit (its end then closes it, even with the hits back as they were).
    pub graph: GraphLane,
    graph_drag: Option<(Vec<Event>, bool)>,
    /// The selected steps, as (pad, step) cells that hold a hit.
    selection: BTreeSet<(u8, usize)>,
    /// The marquee, from its long press: its selection is live.
    boxing: Option<Marquee>,
    /// A touch on the grid until it is decided.
    touch: Option<StepTouch>,
    /// The clip the selection and the box belong to: another clip drops them. The host re-keys it with
    /// [`Self::views`].
    pub(crate) selection_key: Option<ViewKey>,
    /// The time range on the ruler, in content ticks.
    region: RangeSelect,
    /// The floating buttons over the step selection.
    row: FloatingRow,
    /// The drum mixer's fold, toggled by a tap on the current row's header (module docs).
    pub(crate) fold: Fold,
    /// The current row's header is held (its pulse: [`header_pulse`]).
    press: bool,
    /// What new hits take (module docs).
    pub remembered: Remembered,
    /// The cells of hits a tap added in the shown clip, popping in, and the input time each first showed (`None` until
    /// it has). Another clip drops them, except the clip the first tap on an empty lane adds. Each goes within 0.75 s.
    pops: Vec<((u8, usize), Option<f64>)>,
    /// The rows' scroll and zoom controller.
    scroller: Scroller,
    /// The `Paste` button shows, for the press at this point (module docs).
    paste: Option<Pos2>,
    /// The grid cursor: the (pad, step) cell of the last touched hit, where the keymap's `paste` and `open` act
    /// ([`Self::keys`]); `None` before any: step 0 of the selected row.
    cursor: Option<(u8, usize)>,
}

/// A touch on the step grid from its touch-down: where and when, the hit under it, and whether a selection showed
/// then (on empty grid).
#[derive(Clone, Copy, Debug, PartialEq)]
struct StepTouch {
    origin: Pos2,
    since: f64,
    on: Option<(u8, usize)>,
    shown: bool,
}

/// The values new hits take, shared by the step sequencer and the free note-grid: set whenever exactly one hit
/// becomes the selection ([`Self::select`]) and by graph-lane Velocity and Panning edits ([`Self::graph`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remembered {
    /// In beats. Only the free note-grid's new hits use it (a step-mode hit is one step long).
    pub len_beats: f64,
    pub velocity: u8,
    pub pan: u8,
}

impl Default for Remembered {
    /// 1/4 beat, velocity 255, pan centred.
    fn default() -> Self {
        Self {
            len_beats: 0.25,
            velocity: u8::MAX,
            pan: Event::DEFAULT_PAN,
        }
    }
}

impl Remembered {
    /// `hit` alone became the selection: remembers its length (at least `min_len` beats), velocity and pan. Other
    /// events change nothing.
    pub fn select(&mut self, hit: &Event, min_len: f64) {
        if let Event::Drum {
            len_beats,
            velocity,
            pan,
            ..
        } = *hit
        {
            *self = Self {
                len_beats: len_beats.max(min_len),
                velocity,
                pan,
            };
        }
    }

    /// A graph-lane edit set `prop` to `value`: a Velocity or Panning value is remembered.
    pub fn graph(&mut self, prop: Prop, value: u16) {
        let byte = u8::try_from(value).unwrap_or(u8::MAX);
        match prop {
            Prop::Velocity => self.velocity = byte,
            Prop::Pan => self.pan = byte,
            Prop::Pitch | Prop::Repeat => {}
        }
    }
}

/// `k` within step mode's time zoom range.
fn step_k(k: f32) -> f32 {
    k.clamp(STEP_K[0], STEP_K[1])
}

/// The view of a clip with no saved view (step mode): 3 U per beat asked for, then the zoom clamp; posX at the beat of
/// the clip's earliest hit (else 0); scrolled to the lowest used row (the row with the lowest index, at the new rows'
/// pitch; the frame then clamps both scrolls to their ranges).
fn new_view(clip: Option<&Clip>, sig: TimeSig) -> ClipView {
    let events = clip.map_or(&[][..], |c| &c.events);
    let first = events.iter().map(Event::pos).min();
    let row = (events.iter())
        .filter_map(|e| match *e {
            Event::Drum { pad, .. } => Some(pad),
            _ => None,
        })
        .min();
    let rows = RowView::default();
    ClipView {
        rows: RowView {
            scroll: row.map_or(0.0, |r| f32::from(r) * rows.pitch),
            ..rows
        },
        k: step_k(sig.steps_per_beat() as f32 / (4.0 * NEW_BEAT_U)),
        x: first.map_or(0.0, |p| f64::from(p) / BEAT),
    }
}

/// What a frame's rows are painted with ([`StepSeqView::paint_row`]).
struct RowLook<'a> {
    hits: &'a HashMap<(u8, usize), &'a Event>,
    /// L·d: the steps starting before the loop's end.
    loop_steps: f64,
    d: usize,
    /// `controlFill`, `moduleFill`.
    shades: [Color32; 2],
    bg: Color32,
    text: Color32,
    tints: [[Rgba; 3]; 2],
    prop: Prop,
    t: f32,
    now: f64,
}

/// The pop-in's a `elapsed` seconds after a new hit first showed: 1, then times `rate` per 60 Hz frame, 0 once under
/// 0.01. Step mode's rate is [`STEP_POP`]; the free note-grid's 0.6.
pub(crate) fn pop_in(rate: f32, elapsed: f64) -> f32 {
    let a = rate.powf((elapsed.max(0.0) * 60.0) as f32);
    if a < POP_FLOOR { 0.0 } else { a }
}

/// An active step on its `cell` in a row `pitch` points apart: 2 px in from the cell's left and right (so P − 5 px
/// wide) and top, trunc(R − 4) px tall (R the pitch in px), so its bottom is 1 px above the cell's.
pub(crate) fn active_rect(cell: Rect, pitch: f32, ppp: f32) -> Rect {
    let px = 1.0 / ppp;
    let h = (pitch * ppp - 4.0).trunc().max(0.0) * px;
    let top = cell.top() + 2.0 * px;
    Rect::from_x_y_ranges(
        cell.left() + 2.0 * px..=cell.right() - 2.0 * px,
        top..=top + h,
    )
}

/// The `background` dashes on an active step `hit` with repeat `repeat`, `t` the hairline: r + 1 of them, t tall, from
/// 2t in from its left to 2t in from its right; the first 2t below its top, each next 3t lower (not clipped to the hit).
pub(crate) fn dashes(hit: Rect, repeat: u8, t: f32) -> impl Iterator<Item = Rect> {
    (0..=u32::from(repeat)).map(move |i| {
        let top = hit.top() + 2.0 * t + 3.0 * t * i as f32;
        Rect::from_x_y_ranges(hit.left() + 2.0 * t..=hit.right() - 2.0 * t, top..=top + t)
    })
}

/// The drum mixer's expand/collapse: the target (`open`, t = 1) and the width fraction c, stepped once per frame at
/// input time `at` ([`Self::step`]). In memory only: a new view starts collapsed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fold {
    pub open: bool,
    pub c: f32,
    at: f64,
}

impl Fold {
    /// Steps c towards its target for the frame at input time `now`, at most egui's stable `dt` after the last step:
    /// a frame run again at the same time steps nothing, and the first frame after a pause one frame. Returns whether
    /// c is still on its way.
    pub(crate) fn step(&mut self, now: f64, dt: f32) -> bool {
        let t = if self.open { 1.0 } else { 0.0 };
        let dt = (now - self.at).clamp(0.0, f64::from(dt)) as f32;
        self.at = now;
        self.c = fold_step(self.c, t, dt);
        self.c != t
    }
}

/// One step of the fold after `dt` seconds: c ← c + 0.4·(t − c) per frame, counted at 60 frames a second
/// (k = 1 − 0.6^(60·dt)), snapping to t once |c − t| < 0.05.
fn fold_step(c: f32, t: f32, dt: f32) -> f32 {
    let c = c + (1.0 - (1.0 - FOLD_RATE).powf(60.0 * dt)) * (t - c);
    if (c - t).abs() < FOLD_SNAP { t } else { c }
}

/// The current row's header growth in device pixels, held (`down`) or let go: the knobs' touch pulse
/// ([`pulse_growth`]), so the two wobble alike. One pulse per view (`id`): its header and, while held, its row's cells
/// grow by it.
fn header_pulse(ui: &Ui, id: Id, down: bool) -> f32 {
    pulse_growth(ui, id.with("header"), down)
}

/// The row headers in `l`, with ids from `id`: pad name over sample name. The current row's (`selected`) is filled with
/// the channel colour `ink` and its texts are `moduleFill`; it grows while pressed (`press`), over the grid. The others
/// have no fill and channel-coloured texts. The mixer's strip row right of each header is `moduleFill`. The level
/// meters show `levels` (per row, its peaks L and R) and follow the drum mixer's `fold`. A tap on the current row's
/// header folds the drum mixer in or out; on another's, makes it current. Returns the row whose menu a long press (or
/// right click) asked for, and where.
/// The step sequencer and the free note-grid ([`crate::ui::drum_grid`]) each keep their own row, press and fold.
#[allow(clippy::too_many_arguments)]
pub(crate) fn row_headers(
    ui: &Ui,
    painter: &Painter,
    cx: &UiCtx,
    id: Id,
    l: &Layout,
    pads: &dyn DrumPads,
    levels: &[[f32; 2]],
    ink: egui::Color32,
    (current, press, fold): (&mut usize, &mut bool, &mut Fold),
) -> Option<(usize, Pos2)> {
    let (colors, m) = (&cx.theme.colors, cx.m);
    let module_fill = colors.module_fill.to_color32();
    let mut menu = None;
    for row in l.visible_rows() {
        let header = l.header(row);
        let selected = row == *current;
        let mut response = ui.interact(header, id.with(("row", row)), Sense::click());
        let long = gesture::long_press(ui, &mut response, m);
        // The strip row right of the header, under the mixer's page: `moduleFill`, on which an off strip disc is unseen.
        let page = Rect::from_x_y_ranges(l.header_right()..=l.column_right(), header.y_range());
        if page.is_positive() {
            painter.rect_filled(page, 0.0, module_fill);
        }
        if selected {
            *press = response.is_pointer_button_down_on();
            let grow = header_pulse(ui, id, *press);
            painter.rect_filled(header.expand(grow / 2.0 / l.ppp), 0.0, ink);
        }
        let fallback;
        let (name, sample) = if row < pads.count() {
            pads.names(row)
        } else {
            fallback = format!("Pad {}", row + 1);
            (fallback.as_str(), "")
        };
        let color = if selected { module_fill } else { ink };
        // The drum mixer strip's header lines, kept to the header.
        let clip = painter.with_clip_rect(header);
        header_names(&clip, m, header, name, sample, color);
        // The level meters: vertical at the header's edge, horizontal under the page once the mixer is out (along
        // the bottom 5 hairlines of the row, under the strip controls' 75 % high layer).
        let full = fold.c == 1.0;
        let level = levels.get(row).map_or([0.0; 2], |p| p.map(meter_level));
        for bar in meter_bars(header, page, full, level, m.t()) {
            if bar.is_positive() {
                painter.rect_filled(bar, 0.0, if full { ink } else { color });
            }
        }
        // A tap on the current row's header folds the drum mixer in or out; on another's, makes it current.
        if response.clicked() {
            if selected {
                fold.open = !fold.open;
                ui.ctx().request_repaint();
            }
            *current = row;
        }
        if response.clicked_by(egui::PointerButton::Secondary) || long {
            *current = row;
            menu = Some((
                row,
                response.interact_pointer_pos().unwrap_or(header.center()),
            ));
        }
    }
    menu
}

/// The `+` add-drum button in `l`, with id `id`: icon `AddLarge` in `text`, dimmed and inert once the kit's `pads` are
/// all [`PAD_COUNT`] in use (the new pad goes on the next free one). Returns whether it was clicked: the host opens the
/// sample picker ([`StepAction::AddDrum`]).
pub(crate) fn add_drum(
    ui: &Ui,
    painter: &Painter,
    id: Id,
    l: &Layout,
    pads: usize,
    text: egui::Color32,
) -> bool {
    let (rect, full) = (l.add_button(), pads >= PAD_COUNT);
    let ink = if full {
        text.gamma_multiply(DISABLED_ALPHA)
    } else {
        text
    };
    painter.extend(icons::painted_shapes(
        painter.ctx(),
        IconId::AddLarge,
        rect,
        ink,
    ));
    !full && ui.interact(rect, id, Sense::click()).clicked()
}

/// A row's two level-meter bars (L, R) at `levels`, `t` being the hairline (2 hairlines thick, 1 apart).
/// `full` (the mixer fully out): horizontal along the bottom of the `page` area, filling from the left. Otherwise two
/// thin vertical bars at the `header`'s right edge, in the 5 hairlines its text leaves, filling from the bottom.
fn meter_bars(header: Rect, page: Rect, full: bool, levels: [f32; 2], t: f32) -> [Rect; 2] {
    let level = |i: usize| levels[i].clamp(0.0, 1.0);
    // Bar i's band: L 5 to 3 hairlines from the edge, R the last 2.
    let band = |edge: f32, i: usize| {
        let far = edge - if i == 0 { 3.0 * t } else { 0.0 };
        Rangef::new(far - 2.0 * t, far)
    };
    std::array::from_fn(|i| {
        if full {
            let x = Rangef::new(page.left(), page.left() + level(i) * page.width());
            Rect::from_x_y_ranges(x, band(page.bottom(), i))
        } else {
            let y = Rangef::new(
                header.bottom() - level(i) * header.height(),
                header.bottom(),
            );
            Rect::from_x_y_ranges(band(header.right(), i), y)
        }
    })
}

impl StepSeqView {
    /// Paints the view over `ui`'s rect from the drum lane [`Self::lane`] of `song.channels[channel]` and its clip
    /// [`Self::clip`] (a bar of empty steps without that clip, which the steps then never make). Draws nothing when there
    /// is no such drum lane.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        song: &Song,
        channel: usize,
        pads: &dyn DrumPads,
    ) -> Option<StepAction> {
        self.last = None;
        let index = channel;
        let channel = song.channels.get(channel)?;
        let lane = channel
            .lanes
            .get(self.lane)
            .filter(|l| l.kind == LaneKind::Drums)?;
        let clip = lane.clips.iter().find(|c| Some(c.id) == self.clip);
        let (colors, m) = (&cx.theme.colors, cx.m);
        let (text, bg) = (colors.text.to_color32(), colors.background.to_color32());
        let color = channel_color(channel.hue, cx.theme);
        let ink = color.to_color32();
        let rows = edit::row_count(lane, pads.count());
        // d steps a beat (`TDIV` byte 0); the loop L (a lane without a clip shows one bar), and s, the song tick of
        // content tick 0 (`Preview`'s span start).
        let sig = song.time_sig;
        let d = sig.steps_per_beat() as usize;
        let loop_beats = clip.map_or(f64::from(sig.bar_beats()), |c| c.loop_len_beats);
        let origin = region::clip_origin(clip);
        let folded = ruler::folded(self.playhead, origin, loop_beats * BEAT);
        let drawn = ruler::folded(self.playhead + self.lead, origin, loop_beats * BEAT);
        let (rect, graph_rect) = lane_split(ui.max_rect(), m, &self.graph);
        // The drum mixer's fold: the header column is 3 U + c·12 U, and the layout follows it every frame.
        let (now, dt) = ui.input(|i| (i.time, i.stable_dt));
        if self.fold.step(now, dt) {
            ui.ctx().request_repaint();
        }
        self.mixer_w = self.fold.c * m.u(MIXER_W);

        let body = rect.with_min_y(rect.top() + m.u(TOP_H));
        let key = self.key(index, lane);
        let old = self.selection_key.replace(key);
        if old != Some(key) {
            self.selection.clear();
            (self.boxing, self.touch) = (None, None);
            // Another clip's view: a fling of the last one stops, and its pops go.
            self.scroller = Scroller::default();
            self.pops.clear();
        }
        // The clip's view; with none saved, a new clip's (step mode).
        let mut view = (self.views.get(&key).copied()).unwrap_or_else(|| new_view(clip, sig));
        let u = m.u(1.0);
        // The clip's zoom and scroll, by the scroller (step mode): one finger scrolls the rows below the ruler and the
        // time over the grid columns (a touch on the grid until it is a box, a ruler press until its range starts; the
        // headers take only taps), two zoom; the wheel zooms the rows over the header column and the time over the
        // ruler, and scrolls over the grid, where a trackpad pinch zooms the time. The time scrolls over [0, 2L], a
        // beat U·d/(4k) wide with k in [0.1, 0.33]. Follow playhead keeps the folded playhead at a quarter of the view
        // while it plays a loop longer than the view; touch scroll x is off meanwhile.
        let beat_w = |k: f32| drum_grid::edit::beat_w(u, sig.subdivision, step_k(k));
        let laid = Layout::new(rect, rows, 1, m, self.mixer_w, view.rows);
        let k_zoom = |k: f32| 4.0 * f64::from(k) / f64::from(u * d as f32);
        let mut axes = [
            Axis {
                pos: view.x,
                zoom: k_zoom(step_k(view.k)),
                range: [0.0, 2.0 * loop_beats],
                zoom_range: STEP_K.map(k_zoom),
                view: laid.grid().x_range(),
            },
            rows_axis(view.rows, rows, body, u),
        ];
        let was = axes;
        let follow = self.follow && self.playing && loop_beats > axes[X].span();
        let opts = scroller::Options {
            covered: self.covered,
            follow,
        };
        let zones = zones(&laid);
        // A kept view whose loop shrank has posX past the new 2L: the spring brings it back. (A new view, opened at
        // its earliest hit, is clamped at once.)
        if self.views.contains_key(&key) {
            self.scroller.spring_back(X, &axes[X]);
        }
        let report = (self.scroller).update(ui, id().with("scroll"), u, &zones, &mut axes, opts);
        if follow {
            scroller::follow(&mut axes[X], drawn / BEAT);
        }
        if axes[X].zoom != was[X].zoom {
            view.k = step_k((axes[X].zoom * f64::from(u * d as f32) / 4.0) as f32);
        }
        if axes[X] != was[X] {
            view.x = axes[X].pos;
        }
        take_rows(&mut view.rows, was[Y], axes[Y], u);
        let grid = ui.interact(laid.grid(), id().with("grid"), Sense::click_and_drag());
        crate::ui::focus::register(ui, grid.id, grid.rect, crate::ui::focus::Kind::Container);
        let bw = beat_w(view.k);
        let l = Layout::new(rect, rows, 1, m, self.mixer_w, view.rows)
            .scrolled(view.rows.scroll)
            .with_time(laid.grid_left() - view.x as f32 * bw, bw / d as f32);
        view.rows = l.view;
        self.views.insert(key, view);
        self.last = Some(l);
        self.selected = self.selected.min(rows.saturating_sub(1));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, bg);

        // `SWING`: a button opening the swing popup, which starts from the clip's current positions. It has the
        // ruler row's top-left cell, 3 U × (1.5 U − 1 px), to itself (step-sequencer mode: no Snap).
        let strip = l.swing_strip();
        let button = Rect::from_min_size(strip.min, vec2(l.header_w, strip.height() - 1.0 / l.ppp));
        let font = m.font_id(FontSize::Pt10);
        painter.text(button.center(), Align2::CENTER_CENTER, "SWING", font, ink);
        let positions =
            || clip.map_or_else(Vec::new, |c| c.events.iter().map(|e| e.pos()).collect());
        let swing = lane_swing(lane) as f32;
        if ui
            .interact(button, id().with("swing button"), Sense::click())
            .clicked()
        {
            let from = positions();
            self.overlay = Some(Overlay::Swing {
                seen: from.clone(),
                from,
                amount: swing,
                committed: swing,
            });
        }
        if let Some(Overlay::Swing {
            from,
            amount,
            committed,
            seen,
        }) = &mut self.overlay
        {
            let now = positions();
            if *seen != now || *committed != swing {
                (*from, *seen) = (now.clone(), now);
                (*amount, *committed) = (swing, swing);
            }
        }

        // The rows scroll under the strip: everything below is clipped to their area, input included.
        let outer = ui.clip_rect();
        ui.set_clip_rect(outer.intersect(l.body()));
        let painter = ui.painter().clone();

        // The cells and the active steps.
        let hits = clip.map_or_else(HashMap::new, |c| edit::cells(c, sig));
        let is_hit = |cell: (u8, usize)| hits.contains_key(&cell);
        self.selection.retain(|&c| is_hit(c));
        let before = self.selection.clone();
        let mut action = self.grid_input(ui, &grid, &l, is_hit);
        if self.selection != before {
            self.remember_lone(clip.map_or(&[], |c| &c.events), sig);
        }
        let prop = (graph_lane::DRUM_PROPS.get(self.graph.item).copied()).unwrap_or(Prop::Velocity);
        self.pops
            .retain_mut(|(_, at)| pop_in(STEP_POP, now - *at.get_or_insert(now)) > 0.0);
        if !self.pops.is_empty() {
            ui.ctx().request_repaint();
        }
        let look = RowLook {
            hits: &hits,
            loop_steps: loop_beats * d as f64,
            d,
            shades: [colors.control_fill, colors.module_fill].map(|c| c.to_color32()),
            bg,
            text,
            tints: hit_tints(colors, color, prop),
            prop,
            t: m.t(),
            now,
        };
        // While the current row's header is held, that row goes last: its band in `background`, then its cells and
        // hits, all grown by the header's press growth (in height only, so the steps keep their gaps).
        // The cells and hits stay on the grid: a step off content beat 0 starts left of it, under the header column.
        let cells = painter.with_clip_rect(painter.clip_rect().intersect(l.grid()));
        let held = (self.press && l.visible_rows().contains(&self.selected))
            .then(|| header_pulse(ui, id(), true) / l.ppp);
        for row in l.visible_rows() {
            if held.is_none() || row != self.selected {
                self.paint_row(&cells, &l, row, 0.0, &look);
            }
        }
        if let Some(grow) = held {
            let band = l.cell(self.selected, 0).y_range().expand(grow / 2.0);
            let grid = l.grid();
            cells.rect_filled(Rect::from_x_y_ranges(grid.x_range(), band), 0.0, bg);
            self.paint_row(&cells, &l, self.selected, grow, &look);
            ui.ctx().request_repaint();
        }
        // The playhead line at the folded playhead, playing or stopped, from the grid top to the last row's bottom. It
        // lies under the header column, which hides what lies left of its right edge: at beat 0 its left half shows in
        // the t gap.
        // While the knob is dragged, the line and the disc follow the finger: the beat under it, clamped to [the view's
        // left edge, s + L] and snapped to the step; after its drop they stay there until the output reports it
        // (`PlayheadKnob::line_x`).
        let ruler = strip.with_min_x(l.grid_left());
        let axis = TimeAxis {
            origin: l.x0 - (origin as f64 / BEAT) as f32 * bw,
            beat_w: bw,
        };
        let song_end = origin as f64 + loop_beats * BEAT;
        let at = RangeRuler {
            strip: ruler,
            axis,
            grid: Some(sig.step_ticks()),
            sig,
            ink,
            range: (song.play_loop).map(|(start, end)| TimeRange { start, end }),
            playhead: Some(origin as f64 + folded),
        };
        let live = l.x0 + (drawn / BEAT) as f32 * l.step_w * d as f32;
        let line_x = (self.knob).line_x(ui, m, &at, song_end, live);
        let bottom = (l.row_top() + rows as f32 * l.pitch).clamp(l.top, l.rect.bottom());
        let column = Rect::everything_right_of(l.column_right());
        let line = painter.with_clip_rect(painter.clip_rect().intersect(column));
        let span = Rect::from_x_y_ranges(l.grid().x_range(), l.top..=bottom);
        timeline::playhead_line(&line, span, line_x, cx);
        // Row headers: pad name over sample name, and the long press's row menu.
        let heads = (&mut self.selected, &mut self.press, &mut self.fold);
        if let Some((row, at)) =
            row_headers(ui, &painter, cx, id(), &l, pads, &self.levels, ink, heads)
        {
            self.overlay = Some(Overlay::Menu { row, at });
        }

        // The marquee: a square growing while a touch is held on an empty step, then the box.
        let marquee = match (self.boxing, self.touch) {
            (Some(marquee), _) => Some(marquee.drawn(l.u)),
            (None, Some(t)) if t.on.is_none() => {
                ui.ctx().request_repaint();
                hold_square(t.origin, l.u, now - t.since)
            }
            _ => None,
        };
        if let Some(rect) = marquee {
            paint_marquee(&painter, rect, cx);
        }
        // The floating row waits for the box. Its view is the grid and the steps' and rows' zoom and scroll.
        let sel = (self.selection.iter())
            .map(|&(pad, step)| l.cell(usize::from(pad), step))
            .reduce(Rect::union)
            .filter(|_| self.boxing.is_none())
            .map(|rect| Selection {
                rect,
                // Sorted by pad first: one pad iff the first and the last share it.
                one_row: self.selection.first().map(|c| c.0) == self.selection.last().map(|c| c.0),
                handles: &[],
            });
        let shown = GridView {
            grid: l.grid(),
            map: [l.x0, l.row_top(), l.step_w, l.pitch],
        };
        self.row.covered = self.covered;
        if let Some((i, _)) = self.row.show(ui, "step_button", sel, shown, &STEP_OPS, cx) {
            let cells: Vec<_> = self.selection.iter().copied().collect();
            if i == 0 {
                let chosen = |e: &&Event| edit::cell_of(e, sig).is_some_and(|c| cells.contains(&c));
                let hits = clip.map_or_else(Vec::new, |c| {
                    c.events.iter().filter(chosen).copied().collect()
                });
                set_hit_clipboard(ui.ctx(), hits);
            }
            action = Some(match i {
                0 => StepAction::CopySteps { cells },
                _ => StepAction::DeleteSteps { cells },
            });
        }
        if let Some(finger) = self.paste {
            let (row, _) = l.cell_at(finger);
            let band = l.cell(row, 0).y_range();
            let (rect, tap) = paste_button(ui, cx, id().with("paste"), finger, band);
            if tap.is_some() {
                self.paste = None;
            }
            let hits = hit_clipboard(ui.ctx());
            if tap == Some(true) && !hits.is_empty() {
                let step = l.cell_at(rect.center()).1;
                action = Some(StepAction::PasteSteps { step, hits });
            }
        }
        if add_drum(ui, &painter, id().with("add"), &l, pads.count(), text) {
            action = Some(StepAction::AddDrum);
        }
        ui.set_clip_rect(outer);
        let graph = match graph_rect {
            Some(strip) => self.graph_lane(ui, cx, strip, &l, clip, (color, sig, loop_beats)),
            None => {
                (self.graph_drag.take()).and_then(|(_, sent)| sent.then_some(StepAction::GraphEnd))
            }
        };

        // The ruler over the steps in song ticks, its range and region menu, and the playhead knob (module docs).
        let ruler_painter = ui.painter_at(ruler);
        timeline::ruler(&ruler_painter, ruler, ruler, axis.origin, bw, sig, cx);
        let visible = f64::from(ruler.width() / bw);
        ruler::indicator(
            &ruler_painter,
            ruler,
            (view.x, visible, loop_beats),
            ink,
            m.t(),
        );
        if report.scrolled[X] || report.zoomed[X] {
            self.region.view_moved();
        }
        let out = self.region.show(ui, id().with("region"), &at, false, cx);
        // A ruler press waiting for its long press is the scroller's too, until its range starts.
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        if let Some(range) = out.range {
            action = Some(StepAction::PlayLoop(range.map(|r| (r.start, r.end))));
        }
        // A tap seeks only before the loop's end (s + L), and past it does nothing.
        if let Some(tick) = out
            .playhead
            .filter(|&t| loop_beats <= 0.0 || f64::from(t) < song_end)
        {
            action = Some(StepAction::SetPlayhead(tick.into()));
        }
        if let Some((op, range)) = out.pick
            && let Some(range) = TimeRange::in_clip(Some((range.start, range.end)), origin)
        {
            action = Some(StepAction::Region {
                op,
                range,
                playhead: folded as u32,
            });
        }
        // The knob at the folded playhead: its fade's target is stopped, with no floating row; a tap plays; while
        // dragged, the line and the disc follow the finger (`line_x` above), and the drop seeks there.
        self.knob.armed = self.armed;
        let shown = timeline::knob_shown(self.playing, self.row.showing());
        match (self.knob).show(ui, id().with("knob"), ruler, line_x, shown, cx) {
            Some(KnobGesture::Tap) => self.play = true,
            Some(KnobGesture::Drop(x)) => {
                action = Some(StepAction::Seek(self.knob.drop_tick(ui, &at, song_end, x)));
            }
            None => {}
        }

        let out = overlays(
            &mut self.overlay,
            ui.ctx(),
            ui.max_rect(),
            cx,
            rows,
            pads,
            ink,
            true,
        )
        .or(graph)
        .or(action);
        // After the popups, which take Escape first.
        let keys = self.keys(ui, clip, sig);
        out.or(keys)
    }

    /// The keymap's editing actions while the grid holds the keyboard (`ui/edit_keys.rs`), on the selected steps as the
    /// floating row's: `copy` fills the hit clipboard, `cut` too, `cut` and `delete` are [`StepAction::DeleteSteps`],
    /// `paste` is [`StepAction::PasteSteps`] at the cursor's step, `open` adds a hit in the cursor's cell when it is
    /// empty ([`StepAction::Toggle`]), `close` deselects; `selectAll` is the piano roll's only. Each action hides the
    /// floating row and the `Paste` button. No editing action during a touch.
    fn keys(&mut self, ui: &Ui, clip: Option<&Clip>, sig: TimeSig) -> Option<StepAction> {
        let ctx = ui.ctx();
        if self.touch.is_some() || self.boxing.is_some() {
            return None;
        }
        let k = edit_keys::take(ctx)?;
        self.row.hide();
        self.paste = None;
        ctx.request_repaint();
        let cells: Vec<(u8, usize)> = self.selection.iter().copied().collect();
        let hits_in = |cells: &[(u8, usize)]| -> Vec<Event> {
            let chosen = |e: &&Event| edit::cell_of(e, sig).is_some_and(|c| cells.contains(&c));
            clip.map_or_else(Vec::new, |c| {
                c.events.iter().filter(chosen).copied().collect()
            })
        };
        if matches!(k, EditKey::Copy | EditKey::Cut) && !cells.is_empty() {
            set_hit_clipboard(ctx, hits_in(&cells));
        }
        let (pad, step) = self.cursor.unwrap_or((self.selected as u8, 0));
        match k {
            EditKey::Close => {
                self.selection.clear();
                None
            }
            EditKey::Copy | EditKey::SelectAll => None,
            EditKey::Cut | EditKey::Delete => {
                (!cells.is_empty()).then_some(StepAction::DeleteSteps { cells })
            }
            EditKey::Paste => {
                let hits = hit_clipboard(ctx);
                (!hits.is_empty()).then_some(StepAction::PasteSteps { step, hits })
            }
            EditKey::Open => {
                (hits_in(&[(pad, step)]).is_empty()).then_some(StepAction::Toggle { pad, step })
            }
        }
    }

    /// One row's cells and active steps, its band grown by `grow` points in height about its middle:
    /// - every visible step from content beat 0 to the grid's right edge has a cell, 1 px short of the step and the row
    ///   pitch; in the loop (start < L) beats alternate `controlFill` (even) and `moduleFill` (odd), past it every
    ///   cell is `controlFill`;
    /// - a hit sits on its cell, inset 2 px left, right and top, trunc(R − 4) px tall, in the hit colour blended toward
    ///   `text` by its pop-in a, with r + 1 `background` dashes (r its repeat) t tall, 2t in from its sides, the first
    ///   2t below its top and each next 3t lower.
    fn paint_row(&self, painter: &Painter, l: &Layout, row: usize, grow: f32, look: &RowLook) {
        let band = l.cell(row, 0).y_range().expand(grow / 2.0);
        for step in l.visible_steps() {
            let cell = Rect::from_x_y_ranges(l.cell(row, step).x_range(), band);
            let shade = if (step as f64) < look.loop_steps {
                look.shades[(step / look.d) % 2]
            } else {
                look.shades[0]
            };
            painter.rect_filled(cell, 0.0, shade);
            let at = (row as u8, step);
            let Some(&&e) = look.hits.get(&at) else {
                continue;
            };
            let hit = active_rect(cell, l.pitch + grow, l.ppp);
            // The newest pop of the cell (one deleted and added again within the pop's span).
            let a = (self.pops.iter().rev().find(|p| p.0 == at))
                .and_then(|p| p.1)
                .map_or(0.0, |t0| pop_in(STEP_POP, look.now - t0));
            let selected = self.selection.contains(&at);
            let value = look.prop.get(&e).unwrap_or(0);
            let fill = hit_color(&look.tints, look.prop, value, selected);
            painter.rect_filled(hit, 0.0, fill.lerp_to_gamma(look.text, a));
            let repeat = match e {
                Event::Drum { repeat, .. } => repeat,
                _ => 0,
            };
            for dash in dashes(hit, repeat, look.t) {
                painter.rect_filled(dash, 0.0, look.bg);
            }
        }
    }

    /// The graph lane's knob for the view `rect`: at 1.4 U from its left, on the lane's top or the view's bottom edge.
    /// The host calls it after everything it draws over the rows (the drum mixer's strips), so that the knob is on top.
    pub fn graph_knob(&mut self, ui: &mut Ui, cx: &UiCtx, rect: Rect) {
        let m = cx.m;
        let below = (rect, rect.top() + m.u(TOP_H));
        let x = rect.left() + m.u(GRAPH_KNOB_X);
        (self.graph).knob_below(ui, cx, id().with("graph knob"), below, x);
    }

    /// The graph lane in the strip `strip`: its items in the header column, the mixer's room left to the host (the
    /// spacer, `views::drum_editor`), and the value area under the steps: a column per step and the selected row's
    /// hits, which a drag paints step by step. This view is the step-sequencer mode, so `Repeat` is listed. The columns
    /// alternate per beat while they start at or before the loop's end (`loop_beats`), and are all `controlFill` past
    /// it (start ≤ L here, where the grid's cells take start < L).
    fn graph_lane(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        strip: Rect,
        l: &Layout,
        clip: Option<&Clip>,
        (color, sig, loop_beats): (Rgba, TimeSig, f64),
    ) -> Option<StepAction> {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let ink = color.to_color32();
        let labels = &graph_lane::DRUM_LABELS[..graph_lane::drum_items(true)];
        let list = id().with("graph list");
        let item = self.graph.item;
        let content = graph_lane::frame(
            ui,
            cx,
            list,
            strip,
            l.header_w,
            labels,
            ink,
            &mut self.graph.item,
        );
        // The grid drew this frame in the old page's colours.
        if self.graph.item != item {
            ui.ctx().request_repaint();
        }
        let values = content.with_min_x(l.grid_left());
        let prop = graph_lane::DRUM_PROPS[self.graph.item];
        let centred = matches!(prop, Prop::Pan | Prop::Pitch);
        let pad = u8::try_from(self.selected).ok()?;
        let events = clip.map_or(&[][..], |c| &c.events);

        // Columns per step, alternating per beat; the centre band for Panning and Pitch; the row's hits.
        let painter = ui.painter_at(values);
        let px = 1.0 / l.ppp;
        let d = sig.steps_per_beat() as usize;
        for step in l.visible_steps() {
            let x = l.span(l.x0, l.step_w, step);
            let in_loop = step as f64 <= loop_beats * d as f64;
            let fill = if !in_loop || (step / d).is_multiple_of(2) {
                colors.control_fill
            } else {
                colors.module_fill
            };
            painter.rect_filled(
                Rect::from_x_y_ranges(x, values.y_range()),
                0.0,
                fill.to_color32(),
            );
        }
        graph_lane::drum_band(&painter, values, prop, cx);
        let tints = hit_tints(colors, color, prop);
        for e in events {
            let step = edit::step_of(e.pos(), sig);
            let mine = matches!(*e, Event::Drum { pad: p, .. } if p == pad);
            let Some(v) = prop.get(e).filter(|_| mine) else {
                continue;
            };
            let start = l.snap(l.x0 + step as f32 * l.step_w);
            let end = l.snap(l.x0 + (step + 1) as f32 * l.step_w);
            let x = Rangef::new(start + 2.0 * px, (end - 3.0 * px).max(start + 3.0 * px));
            let fill = hit_color(&tints, prop, v, self.selection.contains(&(pad, step)));
            graph_lane::drum_bar(&painter, prop, v, x, values, fill, m);
        }

        // The paint: at touch-down and at every move, the step under x takes the value under y.
        let response = ui.interact(values, id().with("graph"), Sense::drag());
        // A press held over a focus loss is up (see `primary_down_live`): it ends the drag as a release.
        let live = primary_down_live(ui, response.id);
        // A second finger makes a pinch, as over the rows: it ends the drag, putting back what it painted, and paints
        // nothing until the lift.
        let pinch = pinch_press(ui, response.id);
        // What it painted (if anything) put back: a touch that changed nothing.
        if pinch && let Some((orig, _)) = self.graph_drag.take() {
            return Some(StepAction::GraphCancel(orig));
        }
        // Every drag starts afresh, also after one that an undo cancelled (`Context::stop_dragging`) without an end.
        if response.drag_started() && live && !pinch {
            self.graph_drag = Some((events.to_vec(), false));
        }
        if response.dragged()
            && live
            && let Some(p) = response.interact_pointer_pos()
            && self.graph_drag.is_some()
        {
            let at = (p.x - l.x0) / l.step_w;
            let v = if centred {
                graph_lane::v_centred(values, p.y, m)
            } else {
                graph_lane::v_at(values, p.y)
            };
            let (step, value) = (at as usize, prop.stored(v));
            // Sent only when it changes an eligible hit (the row's, and while steps are selected a selected one).
            let eligible = self.selection.is_empty() || self.selection.contains(&(pad, step));
            let changes = at >= 0.0
                && eligible
                && events.iter().any(|e| {
                    matches!(*e, Event::Drum { pad: p, .. } if p == pad)
                        && edit::step_of(e.pos(), sig) == step
                        && prop.get(e) != Some(value)
                });
            if changes && let Some((_, sent)) = self.graph_drag.as_mut() {
                *sent = true;
                return Some(StepAction::Graph {
                    pad,
                    prop,
                    step,
                    value,
                });
            }
        }
        if (response.drag_stopped() || !live)
            && let Some((orig, sent)) = self.graph_drag.take()
        {
            // A gesture that changes nothing clears the selection, makes no undo step and re-syncs (its own undo).
            if events == orig.as_slice() {
                self.selection.clear();
                return Some(StepAction::GraphCancel(orig));
            }
            return sent.then_some(StepAction::GraphEnd);
        }
        None
    }

    /// Touches and right clicks on the grid, and the selection box (module docs). A tap that toggles its step returns
    /// it.
    fn grid_input(
        &mut self,
        ui: &Ui,
        grid: &Response,
        l: &Layout,
        is_hit: impl Fn((u8, usize)) -> bool,
    ) -> Option<StepAction> {
        let (held, pressed, pos, now) = ui.input(|i| {
            let p = &i.pointer;
            // Unlike `latest_pos`, `interact_pos` is kept through a touch's lift.
            (
                p.primary_down(),
                p.primary_pressed(),
                p.interact_pos(),
                i.time,
            )
        });
        // A press held over a focus loss is up (see `primary_down_live`): it ends the touch or the box as a release, and
        // starts nothing.
        let down = primary_down_live(ui, grid.id);
        let stale = held && !down;
        let pos = pos.filter(|_| !stale);
        // Two fingers are a pinch: no box and no tap.
        let pinch = ui.input(|i| i.multi_touch().is_some());
        if pinch {
            (self.boxing, self.touch) = (None, None);
        }
        let cell = |p: Option<Pos2>| p.and_then(|p| l.hit(p)).map(|(row, s)| (row as u8, s));
        let clipboard = || !hit_clipboard(ui.ctx()).is_empty();
        let mut action = None;
        if pressed && grid.is_pointer_button_down_on() {
            self.paste = None;
        }
        // Touch-down on a hit: it becomes the whole selection, unless it lands inside the showing selection's frame.
        if pressed
            && !pinch
            && grid.is_pointer_button_down_on()
            && let Some(p) = pos
        {
            let frame = (self.selection.iter())
                .map(|&(pad, step)| l.cell(usize::from(pad), step))
                .reduce(Rect::union);
            let on = cell(Some(p)).filter(|&c| is_hit(c));
            self.cursor = on.or(self.cursor);
            // The keymap's `editorSelect` (Ctrl/Cmd held): a marquee at once, wherever the touch lands.
            let select = crate::keymap::held(ui.ctx(), "editorSelect");
            if let Some(c) = on.filter(|_| !select)
                && !frame.is_some_and(|f| f.contains(p))
            {
                self.selection = BTreeSet::from([c]);
            }
            if select || on.is_none() && ui.input(|i| i.modifiers.shift) {
                // The selection modifier drags a marquee at once.
                self.boxing = Some(Marquee::at(p, true));
            } else {
                self.touch = Some(StepTouch {
                    origin: p,
                    since: now,
                    on,
                    shown: !self.selection.is_empty() && on.is_none(),
                });
                // Until it is a box, the touch is the scroller's too (a drag scrolls the rows).
                self.scroller.claim();
            }
        }
        if let Some(t) = self.touch.take() {
            // Out of the scroller's calm zone on either axis.
            let moved = (self.scroller.left_calm())
                .unwrap_or_else(|| gesture::left_calm(t.origin, pos, l.u));
            let long = gesture::is_long(now - t.since);
            // Held past a tap inside the calm zone: a long press (or the box it may start), no scroll.
            if long && !moved {
                self.scroller.yield_touch();
            }
            match pos {
                // Held, then moved: a box (on an empty step it starts as the hold comes due). Moved sooner: a drag,
                // which scrolls the rows.
                Some(_) if moved && long => self.boxing = Some(Marquee::at(t.origin, false)),
                _ if moved => {}
                _ if down && long && t.on.is_none() => {
                    self.boxing = Some(Marquee::at(t.origin, false))
                }
                _ if down => {
                    if !long {
                        // The long press needs a frame once it comes due.
                        ui.ctx().request_repaint_after_secs(
                            (gesture::LONG_PRESS_S - (now - t.since)) as f32,
                        );
                    }
                    self.touch = Some(t);
                }
                // A long press shows the touch-down's selection (on an empty step it is the box, above); a stale
                // press does nothing.
                _ if long || stale => {}
                _ => match t.on {
                    // A tap on a hit deletes it when it alone is selected, else only deselects.
                    Some((pad, step)) if self.selection.len() == 1 => {
                        self.selection.clear();
                        action = Some(StepAction::Toggle { pad, step });
                    }
                    Some(_) => self.selection.clear(),
                    None if t.shown => self.selection.clear(),
                    None => {
                        action = cell(Some(t.origin))
                            .map(|(pad, step)| StepAction::Toggle { pad, step });
                    }
                },
            }
        }
        // A right click on a hit makes it the selection; on an empty step it is `Paste`, while the clipboard is set.
        if grid.secondary_clicked()
            && !down
            && let Some(p) = grid.interact_pointer_pos()
        {
            match cell(Some(p)).filter(|&c| is_hit(c)) {
                Some(c) => self.selection = BTreeSet::from([c]),
                None if clipboard() => self.paste = Some(p),
                None => {}
            }
        }
        // The box follows the pointer until the button is up (egui's drag does not outlive a touch long press); the
        // hits of the steps it touches are the selection, live.
        if let Some(marquee) = &mut self.boxing {
            self.scroller.yield_touch();
            marquee.follow(pos, l.u);
            let region = marquee.region(l.grid());
            let ((r0, s0), (r1, s1)) = (l.cell_at(region.min), l.cell_at(region.max));
            let cells = (r0.min(r1)..=r0.max(r1))
                .flat_map(|row| (s0.min(s1)..=s0.max(s1)).map(move |s| (row as u8, s)));
            self.selection = cells.filter(|&c| is_hit(c)).collect();
            if !down {
                // Lifted in place: `Paste`, while the clipboard is set.
                if !marquee.dragged && clipboard() {
                    self.paste = Some(marquee.from);
                }
                self.boxing = None;
            }
        }
        action
    }

    /// Step `step`'s cell in row `row` as the last frame laid it out.
    #[cfg(test)]
    pub(crate) fn cell_rect(&self, row: usize, step: usize) -> Option<Rect> {
        self.last.map(|l| l.cell(row, step))
    }

    /// The last frame's x of content beat 0 and step width, in points.
    #[cfg(test)]
    pub(crate) fn time_axis(&self) -> Option<(f32, f32)> {
        self.last.map(|l| (l.x0, l.step_w))
    }

    /// The last frame's top of row 0 (above the rows' area once scrolled) and row pitch, in points: the host lines
    /// the drum mixer's strips up with the rows. `None` when the view drew nothing.
    pub(crate) fn row_band(&self) -> Option<(f32, f32)> {
        self.last.map(|l| (l.top - l.scroll, l.pitch))
    }

    /// The selection changed: when it holds exactly one hit, that hit is remembered (step mode has no minimum length).
    fn remember_lone(&mut self, events: &[Event], sig: TimeSig) {
        let mut chosen = (events.iter())
            .filter(|e| edit::cell_of(e, sig).is_some_and(|c| self.selection.contains(&c)));
        if let (Some(e), None) = (chosen.next(), chosen.next()) {
            self.remembered.select(e, 0.0);
        }
    }

    /// The [`ViewKey`] of the clip shown from `lane` of channel `channel`.
    fn key(&self, channel: usize, lane: &Lane) -> ViewKey {
        let clip = lane.clips.iter().find(|c| Some(c.id) == self.clip);
        (channel, self.lane, clip.map(|c| c.id))
    }

    /// Feeds a MIDI key to the view: while `Set MIDI Note` waits, routes it to that pad and closes the prompt.
    pub fn midi_key(&mut self, note: u8) -> Option<StepAction> {
        let Some(Overlay::LearnMidi(row)) = self.overlay else {
            return None;
        };
        self.overlay = None;
        Some(StepAction::SetMidiNote {
            pad: row as u8,
            note: Some(note),
        })
    }

    /// Makes `action`'s edit on the drum-lane events of `song.channels[channel]`. Without the view's clip the clip edits
    /// do nothing: no clip is ever made here. `Rename` and `SetMidiNote` change nothing here (kit only).
    pub fn apply(&mut self, song: &mut Song, channel: usize, action: &StepAction) {
        match *action {
            StepAction::PlayLoop(play_loop) => song.play_loop = play_loop,
            StepAction::Region { op, .. } if !op.keeps_loop() => song.play_loop = None,
            _ => {}
        }
        let time_sig = song.time_sig;
        let Some(channel) = song.channels.get_mut(channel) else {
            return;
        };
        self.apply_to(channel, time_sig, action);
    }

    fn apply_to(&mut self, channel: &mut Channel, sig: TimeSig, action: &StepAction) {
        match *action {
            StepAction::Clone { pad } => {
                // Both row edits move rows, and the selection's cells with them: it goes.
                self.selection.clear();
                edit::clone_row(&mut channel.lanes, pad);
                self.selected = usize::from(pad) + 1;
                return;
            }
            StepAction::Delete { pad } => {
                self.selection.clear();
                edit::delete_row(&mut channel.lanes, pad);
                // Every drum clip it took hits from fits its loop (below).
                let drums = channel
                    .lanes
                    .iter_mut()
                    .filter(|l| l.kind == LaneKind::Drums);
                for clip in drums.flat_map(|l| &mut l.clips) {
                    clip.fit_loop(sig.bar_beats());
                }
                if self.selected > usize::from(pad) {
                    self.selected -= 1;
                }
                return;
            }
            StepAction::Rename { .. } | StepAction::SetMidiNote { .. } | StepAction::GraphEnd => {
                return;
            }
            _ => {}
        }
        let Some(lane) = channel.lanes.get_mut(self.lane) else {
            return;
        };
        if let StepAction::Swing { amount, from, .. } = action {
            lane.swing = *amount;
            if let Some(clip) = lane.clips.iter_mut().find(|c| Some(c.id) == self.clip) {
                edit::swing(clip, from, sig, *amount);
                clip.fit_loop(sig.bar_beats());
                // The popup's own edit, not an outside change.
                if let Some(Overlay::Swing { seen, .. }) = &mut self.overlay {
                    *seen = clip.events.iter().map(|e| e.pos()).collect();
                }
            }
            return;
        }
        let swing = (sig, lane_swing(lane));
        let Some(clip) = lane.clips.iter_mut().find(|c| Some(c.id) == self.clip) else {
            return;
        };
        let bar = sig.bar_beats();
        match action {
            &StepAction::Toggle { pad, step } => {
                let Remembered { velocity, pan, .. } = self.remembered;
                // The pop goes on the new hit's own cell: Swing may put it on the next step.
                if let Some(cell) = edit::toggle(clip, pad, step, velocity, pan, swing) {
                    self.pops.push((cell, None));
                }
            }
            &StepAction::Fill { pad, every } => edit::fill(clip, pad, every, sig),
            &StepAction::Clear { pad } => edit::clear(clip, pad),
            &StepAction::Rotate { pad, right } => edit::rotate(clip, pad, right, swing),
            StepAction::CopySteps { cells } => {
                self.selection = edit::copy_steps(clip, cells, sig).into_iter().collect();
                self.remember_lone(&clip.events, sig);
            }
            &StepAction::Region {
                op,
                range,
                playhead,
            } => {
                region::edit_clip(clip, op, range, playhead, bar);
                self.selection.clear();
            }
            &StepAction::Graph {
                pad,
                prop,
                step,
                value,
            } => {
                // While steps are selected, only they are eligible.
                if self.selection.is_empty() || self.selection.contains(&(pad, step)) {
                    edit::set_prop(clip, pad, step, prop, value, sig);
                    self.remembered.graph(prop, value);
                }
            }
            StepAction::DeleteSteps { cells } => {
                edit::delete_steps(clip, cells, sig);
                self.selection.clear();
            }
            StepAction::PasteSteps { step, hits } => {
                self.selection = edit::paste_steps(clip, hits, *step, sig)
                    .into_iter()
                    .collect();
                self.remember_lone(&clip.events, sig);
            }
            StepAction::GraphCancel(orig) => clip.events.clone_from(orig),
            _ => {}
        }
        // Every hit edit fits the loop to the hits (recomputed on every clip update), as the row menu's `Delete` and
        // Swing do above.
        if matches!(
            action,
            StepAction::Toggle { .. }
                | StepAction::Fill { .. }
                | StepAction::Clear { .. }
                | StepAction::Rotate { .. }
                | StepAction::CopySteps { .. }
                | StepAction::DeleteSteps { .. }
                | StepAction::PasteSteps { .. }
        ) {
            clip.fit_loop(bar);
        }
    }

    /// Closes the popups over the view (the row menu and its prompts, the swing popup, the ruler's region menu and
    /// `+`): the editor stopped showing. A graph-lane drag under way ends with it. Returns whether that drag had sent
    /// edits: the host closes their undo step (a final edit), as its release would.
    pub(crate) fn close_popups(&mut self) -> bool {
        (self.overlay, self.paste) = (None, None);
        self.region.close_popups();
        self.graph_drag.take().is_some_and(|(_, sent)| sent)
    }

    /// The transport's `Lock` button (drum view only), centred on `center`: an outline circle with `Lock` in it,
    /// filled while on. A click toggles [`Self::lock`].
    pub fn lock_button(&mut self, ui: &mut Ui, cx: &UiCtx, center: Pos2) {
        let colors = &cx.theme.colors;
        let (text, bg) = (colors.text.to_color32(), colors.background.to_color32());
        let d = cx.m.u(LOCK_D);
        if outline_circle_button(ui, id().with("lock"), center, d, text, cx.m).clicked() {
            self.lock = !self.lock;
        }
        if self.lock {
            ui.painter().circle_filled(center, d / 2.0, text);
        }
        let ink = if self.lock { bg } else { text };
        let font = cx.m.font_id(FontSize::Pt8);
        ui.painter()
            .text(center, Align2::CENTER_CENTER, "Lock", font, ink);
    }
}

/// Shows the open `overlay` over the view `rect` and returns what it produced. `rotate`: the row menu lists `Rotate
/// right` / `Rotate left` (step-sequencer mode only; the free note-grid hides them).
#[allow(clippy::too_many_arguments)]
pub(crate) fn overlays(
    overlay: &mut Option<Overlay>,
    ctx: &Context,
    rect: Rect,
    cx: &UiCtx,
    rows: usize,
    pads: &dyn DrumPads,
    ink: egui::Color32,
    rotate: bool,
) -> Option<StepAction> {
    let midi = |row: usize| (row < pads.count()).then(|| pads.midi_note(row)).flatten();
    match overlay.take()? {
        Overlay::Menu { row, at } => {
            let mut items = row_menu(rows, midi(row));
            if !rotate {
                items.retain(|i| !matches!(i.id, Some(RowItem::RotateRight | RowItem::RotateLeft)));
            }
            let result = popup::show(ctx, id().with("row menu"), at, rect, &items, cx);
            if !result.close {
                *overlay = Some(Overlay::Menu { row, at });
            }
            pick(overlay, result.selected?, row, pads)
        }
        Overlay::ConfirmDelete(row) => {
            let items = [
                PopupItem::header(0, 0, DELETE_PROMPT),
                PopupItem::new(true, 0, 1, "Yes"),
                PopupItem::new(false, 0, 2, "No"),
            ];
            let result = popup::show(ctx, id().with("delete"), rect.center(), rect, &items, cx);
            if !result.close {
                *overlay = Some(Overlay::ConfirmDelete(row));
            }
            (result.selected == Some(true)).then_some(StepAction::Delete { pad: row as u8 })
        }
        Overlay::LearnMidi(row) => {
            let items = [
                PopupItem::header(0, 0, MIDI_PROMPT),
                PopupItem::new((), 0, 1, "Cancel"),
            ];
            let result = popup::show(ctx, id().with("midi"), rect.center(), rect, &items, cx);
            if !result.close {
                *overlay = Some(Overlay::LearnMidi(row));
            }
            None
        }
        Overlay::Rename { row, mut text } => {
            let result = name_prompt(ctx, id().with("rename"), rect, &mut text, cx);
            let action = result.selected.map(|()| StepAction::Rename {
                pad: row as u8,
                name: text.clone(),
            });
            if !result.close {
                *overlay = Some(Overlay::Rename { row, text });
            }
            action
        }
        Overlay::Swing {
            from,
            mut amount,
            mut committed,
            seen,
        } => {
            let (close, at_rest) = swing_popup(
                ctx,
                rect.min + vec2(0.0, cx.m.u(TOP_H)),
                &mut amount,
                ink,
                cx,
            );
            // Applied once per release (or double-click reset, or close mid-drag), always from the opening
            // snapshot.
            let action = ((at_rest || close) && amount != committed).then(|| {
                committed = amount;
                StepAction::Swing {
                    amount: f64::from(amount),
                    from: from.clone(),
                    final_edit: true,
                }
            });
            if !close {
                *overlay = Some(Overlay::Swing {
                    from,
                    amount,
                    committed,
                    seen,
                });
            }
            action
        }
    }
}

/// The row-menu item `item` picked on `row`: an action, or the next overlay.
fn pick(
    overlay: &mut Option<Overlay>,
    item: RowItem,
    row: usize,
    pads: &dyn DrumPads,
) -> Option<StepAction> {
    let pad = row as u8;
    let next = match item {
        RowItem::LoadSample => return Some(StepAction::LoadSample { pad }),
        RowItem::LoadDrumset => return Some(StepAction::LoadDrumset),
        RowItem::SaveDrumset => return Some(StepAction::SaveDrumset),
        RowItem::Clone => return Some(StepAction::Clone { pad }),
        RowItem::ResetMidiNote => return Some(StepAction::SetMidiNote { pad, note: None }),
        RowItem::Fill(every) => return Some(StepAction::Fill { pad, every }),
        RowItem::Clear => return Some(StepAction::Clear { pad }),
        RowItem::RotateRight => return Some(StepAction::Rotate { pad, right: true }),
        RowItem::RotateLeft => return Some(StepAction::Rotate { pad, right: false }),
        RowItem::Delete => Overlay::ConfirmDelete(row),
        RowItem::SetMidiNote => Overlay::LearnMidi(row),
        RowItem::SetName => Overlay::Rename {
            row,
            text: if row < pads.count() {
                pads.custom_name(row).to_owned()
            } else {
                String::new()
            },
        },
    };
    *overlay = Some(next);
    None
}

/// The row-menu items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowItem {
    LoadSample,
    LoadDrumset,
    SaveDrumset,
    Clone,
    Delete,
    /// Stores no note (−1): picking it returns `StepAction::SetMidiNote { note: None }`. Shows only for a pad with a
    /// stored note (`DrumPads::midi_note`, the kit's `SamplerSlot::midi_note` / `SlotFileState.midi_key`); a pad
    /// without one gets [`Self::SetMidiNote`] in its place (`row_menu`).
    ResetMidiNote,
    /// Routes the next MIDI-controller note-on to the pad ([`StepSeqView::midi_key`]). Waits on MIDI input, which
    /// the app has none of: the prompt shows and waits until cancelled.
    SetMidiNote,
    SetName,
    Fill(usize),
    Clear,
    RotateRight,
    RotateLeft,
}

/// The row menu: column 0 the sample and kit items, column 1 the pattern items. `Load Sample`,
/// `Load Drumset` and `Save Drumset` ask the host ([`StepAction::LoadSample`] …). `Delete` needs a second row,
/// `Reset MIDI Note` (the pad has a note) and `Set MIDI Note` (it has none) share one row, and the rotations keep
/// the menu open. The rotations always show: this view is the step-sequencer mode.
///
/// `Clone` needs a free pad, so it is disabled once the rows (the kit's slots, or hits on higher pads) fill the
/// [`PAD_COUNT`] pads: the copy goes on the next pad, and past the last one it would sit on no hexagon and could not be
/// saved (`RSMP` holds 22 pads).
fn row_menu(rows: usize, midi_note: Option<u8>) -> Vec<PopupItem<RowItem>> {
    use RowItem::*;
    let item = |id, label| PopupItem::new(id, 0, 0, label);
    let off = |id, label, disabled| PopupItem {
        disabled,
        ..item(id, label)
    };
    let mut kit = vec![
        item(LoadSample, "Load Sample"),
        item(LoadDrumset, "Load Drumset"),
        item(SaveDrumset, "Save Drumset"),
        off(Clone, "Clone", rows >= PAD_COUNT),
        off(Delete, "Delete", rows < 2),
    ];
    kit.extend([
        match midi_note {
            Some(_) => item(ResetMidiNote, "Reset MIDI Note"),
            None => item(SetMidiNote, "Set MIDI Note"),
        },
        item(SetName, "Set Name"),
    ]);
    let keep = |id, label| PopupItem {
        keep_open: true,
        ..item(id, label)
    };
    let pattern = vec![
        item(Fill(1), "Fill every step"),
        item(Fill(2), "Fill each 2 step"),
        item(Fill(4), "Fill each 4 step"),
        item(Clear, "Clear"),
        keep(RotateRight, "Rotate right"),
        keep(RotateLeft, "Rotate left"),
    ];
    let place = |col: usize, items: Vec<PopupItem<RowItem>>| {
        items
            .into_iter()
            .enumerate()
            .map(move |(row, it)| PopupItem { col, row, ..it })
    };
    place(0, kit).chain(place(1, pattern)).collect()
}

/// The lane's Swing amount as the view uses it: clamped to [0, 1], and NaN (never equal to itself, so the popup would
/// apply it every frame) is 0.
fn lane_swing(lane: &Lane) -> f64 {
    if lane.swing.is_nan() {
        0.0
    } else {
        lane.swing.clamp(0.0, 1.0)
    }
}

/// The view `rect` split into the part above the graph lane (the `SWING` strip and the rows) and the lane's strip,
/// if it shows, below the strip: the lane sizes against the view below its top window margin, which is 0 here (the
/// strip counts, as the roll's: [`GraphLane::split_below`]).
pub(crate) fn lane_split(rect: Rect, m: &Metrics, lane: &GraphLane) -> (Rect, Option<Rect>) {
    let (above, strip) = lane.split_below(rect, rect.top() + m.u(TOP_H), m);
    (rect.with_max_y(above.bottom()), strip)
}

/// As in `popup::show`: a full-screen backdrop just below the modal `id` takes every click outside it. Returns
/// whether one happened.
fn backdrop(ctx: &Context, id: Id) -> bool {
    let response = crate::ui::backdrop(ctx, id);
    response.clicked() || response.secondary_clicked()
}

/// The swing popup at `top_left`: a `menuFill` box holding the slider (`Swing` over the track over the % value),
/// with no parameter menu. Escape or a click outside closes it. Returns (close, the slider is at rest), where at
/// rest means not being dragged: a changed value at rest is ready to apply.
fn swing_popup(
    ctx: &Context,
    top_left: Pos2,
    amount: &mut f32,
    ink: egui::Color32,
    cx: &UiCtx,
) -> (bool, bool) {
    let m = cx.m;
    let id = id().with("swing");
    let rect = Rect::from_min_size(top_left, vec2(m.u(SWING_POPUP[0]), m.u(SWING_POPUP[1])));
    let escape = crate::ui::close_pressed(ctx);
    let close = backdrop(ctx, id) || escape;
    let at_rest = Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            ui.painter()
                .rect_filled(rect, 0.0, cx.theme.colors.menu_fill.to_color32());
            let control = Control {
                value_text: true,
                menu: ParamMenu::Off,
                ..Control::new(
                    id.with("slider"),
                    rect.shrink(m.u(SWING_INSET)),
                    "Swing",
                    ink,
                    0.0,
                )
            };
            let response = h_slider(ui, cx, &control, amount, |v| format!("{:.0}%", v * 100.0));
            !response.dragged()
        })
        .inner;
    (close, at_rest)
}

/// The `Set Name` prompt: a modal box in the popup style (`text` fill, `background` ink) with the title and a
/// one-line field, centred in `bounds`. Enter selects (`Some(())`) and closes; Escape or a click outside closes.
fn name_prompt(
    ctx: &Context,
    id: Id,
    bounds: Rect,
    text: &mut String,
    cx: &UiCtx,
) -> PopupResult<()> {
    let m = cx.m;
    let (fill, ink) = (
        cx.theme.colors.text.to_color32(),
        cx.theme.colors.background.to_color32(),
    );
    let row_h = m.u(PROMPT_ROW_H);
    let rect = Rect::from_center_size(bounds.center(), vec2(m.u(PROMPT_W), 2.0 * row_h));
    let mut result = PopupResult {
        selected: None,
        close: backdrop(ctx, id),
    };
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(rect.size(), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, fill);
            let pad = m.u(PROMPT_PAD);
            ui.painter().text(
                pos2(rect.left() + pad, rect.top() + row_h / 2.0),
                Align2::LEFT_CENTER,
                RENAME_TITLE,
                m.font_id(FontSize::Pt12),
                ink,
            );
            let field = Rect::from_x_y_ranges(
                rect.left() + pad..=rect.right() - pad,
                Rangef::new(rect.top() + row_h, rect.bottom()),
            );
            let visuals = ui.visuals_mut();
            visuals.text_cursor.stroke.color = ink;
            visuals.selection.bg_fill = ink.gamma_multiply(0.3);
            visuals.selection.stroke.color = ink;
            let edit = egui::TextEdit::singleline(text)
                .id(id.with("field"))
                .frame(false)
                .text_color(ink)
                .font(m.font_id(FontSize::Pt10))
                .desired_width(field.width());
            let response = ui.put(field, edit);
            if response.lost_focus() {
                result.close = true;
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    result.selected = Some(());
                }
            } else if !response.has_focus() {
                // The cursor at the end of the name it opens with, not where the last prompt left it.
                let mut state = TextEditState::load(ui.ctx(), response.id).unwrap_or_default();
                let end = CCursor::new(text.chars().count());
                state.cursor.set_char_range(Some(CCursorRange::one(end)));
                state.store(ui.ctx(), response.id);
                response.request_focus();
            }
        });
    result
}

/// Where everything sits for a view `rect` with `rows` rows, at a clip's zoom and scroll. Row i starts at the rows'
/// top + i·pitch − scroll; step k at x0 + k·step_w ([`Self::with_time`]; until then `steps` steps share the grid's
/// width from its left, as the free grid has them). Row and step edges are snapped to whole device pixels (a pixel is
/// covered when an unrounded quad covers its centre); each row and step is one device pixel short of its pitch, so the
/// background shows between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Layout {
    pub(crate) rect: Rect,
    pub(crate) ppp: f32,
    pub(crate) header_w: f32,
    /// The drum mixer's room between the headers and the grid.
    pub(crate) mixer_w: f32,
    /// The gap between the header column and the grid: t = ceil(U/16) px, in points.
    pub(crate) t: f32,
    /// The top of the rows' area, under the `SWING` strip.
    pub(crate) top: f32,
    /// The clip's zoom and scroll, the scroll clamped to its range.
    pub(crate) view: RowView,
    /// The row pitch and the scroll, in points.
    pub(crate) pitch: f32,
    pub(crate) scroll: f32,
    pub(crate) u: f32,
    /// The x of content beat 0, and a step's width, in points.
    pub(crate) x0: f32,
    pub(crate) step_w: f32,
    pub(crate) rows: usize,
}

/// The drum editor's input zones in layout `l`: touch scroll y below the ruler across the full width, header column
/// included; touch scroll x over the grid columns from the ruler's top; the wheel zooms the rows over the header column
/// (the drum mixer's room included) and the time over the ruler, and scrolls over the grid, where a trackpad pinch
/// zooms the time. Both drum modes have a time axis. The grid's zones start at GL ([`Layout::grid_left`]). The rects
/// are as laid out (no whole-pixel truncation), and the editor's bottom is the rows' (the graph lane takes its own
/// touches).
pub(crate) fn zones(l: &Layout) -> Zones {
    let (r, top, left) = (l.rect, l.top, l.grid_left());
    let rows = r.with_min_y(top);
    let headers = rows.with_max_x(r.left() + l.header_w + l.mixer_w);
    let grid = rows.with_min_x(left);
    let ruler = Rect::from_x_y_ranges(left..=r.right(), r.top()..=top);
    Zones {
        touch: [r.with_min_x(left), rows],
        wheel_zoom: [ruler, headers],
        wheel_scroll: grid,
        pinch: grid,
    }
}

/// A clip's rows as the scroller's y axis, in rows over the rows' area `body`: they scroll over every row and 1.5
/// more, their pitch 2 U … 100/24 U.
pub(crate) fn rows_axis(view: RowView, rows: usize, body: Rect, u: f32) -> Axis {
    let pitch = f64::from(clamp_pitch(view.pitch) * u);
    Axis {
        pos: f64::from(view.scroll * u) / pitch,
        zoom: 1.0 / pitch,
        range: [0.0, rows as f64 + f64::from(ROOM_ROWS)],
        zoom_range: [PITCH_MAX, PITCH_MIN].map(|p| 1.0 / f64::from(p * u)),
        view: body.y_range(),
    }
}

/// Takes the rows' axis back into `view` where it changed from `was` ([`rows_axis`]).
pub(crate) fn take_rows(view: &mut RowView, was: Axis, y: Axis, u: f32) {
    if y.zoom != was.zoom {
        view.pitch = (1.0 / y.zoom) as f32 / u;
    }
    if y != was {
        view.scroll = (y.pos / y.zoom) as f32 / u;
    }
}

impl Layout {
    pub(crate) fn new(
        rect: Rect,
        rows: usize,
        steps: usize,
        m: &Metrics,
        mixer_w: f32,
        view: RowView,
    ) -> Self {
        let (u, top) = (m.u(1.0), rect.top() + m.u(TOP_H));
        let header_w = m.u(HEADER_W);
        // GL = x + HW + ceil(U/16) px.
        let t = m.t();
        let left = header_w + mixer_w + t;
        let pitch = clamp_pitch(view.pitch);
        let range = ((rows as f32 + ROOM_ROWS) * pitch - (rect.bottom() - top) / u).max(0.0);
        let view = RowView {
            pitch,
            scroll: view.scroll.clamp(0.0, range),
        };
        Self {
            rect,
            ppp: m.ppp(),
            header_w,
            mixer_w,
            t,
            top,
            view,
            pitch: pitch * u,
            scroll: view.scroll * u,
            u,
            x0: rect.left() + left,
            step_w: (rect.width() - left) / steps.max(1) as f32,
            rows,
        }
    }

    /// The layout with content beat 0 at `x0` and steps `step_w` points wide: the step grid's time zoom and scroll.
    pub(crate) fn with_time(self, x0: f32, step_w: f32) -> Self {
        Self { x0, step_w, ..self }
    }

    /// The steps at least partly in view, from content beat 0 (nothing is drawn left of it) to the grid's right edge.
    pub(crate) fn visible_steps(&self) -> std::ops::Range<usize> {
        let at = |x: f32| (x - self.x0) / self.step_w;
        // `as` saturates: a step left of content beat 0 is step 0.
        at(self.grid_left()).floor() as usize..at(self.rect.right()).ceil() as usize
    }

    /// The layout scrolled to `scroll` U, past either end of its range while the scroller pulls or springs it.
    pub(crate) fn scrolled(mut self, scroll: f32) -> Self {
        self.view.scroll = scroll;
        self.scroll = scroll * self.u;
        self
    }

    pub(crate) fn snap(&self, v: f32) -> f32 {
        (v * self.ppp).round() / self.ppp
    }

    pub(crate) fn snap_rect(&self, r: Rect) -> Rect {
        Rect::from_min_max(
            pos2(self.snap(r.left()), self.snap(r.top())),
            pos2(self.snap(r.right()), self.snap(r.bottom())),
        )
    }

    pub(crate) fn header_right(&self) -> f32 {
        self.rect.left() + self.header_w
    }

    /// The header column's right edge, the drum mixer's room included: what lies left of it is the column's.
    pub(crate) fn column_right(&self) -> f32 {
        self.header_right() + self.mixer_w
    }

    /// GL: the grid's left, a t gap right of the header column.
    pub(crate) fn grid_left(&self) -> f32 {
        self.column_right() + self.t
    }

    pub(crate) fn swing_strip(&self) -> Rect {
        self.rect.with_max_y(self.top)
    }

    /// Everything below the strip: the rows, the drum mixer's room and the grid.
    pub(crate) fn body(&self) -> Rect {
        self.rect.with_min_y(self.top)
    }

    /// The top of row 0, scrolled.
    pub(crate) fn row_top(&self) -> f32 {
        self.top - self.scroll
    }

    /// The rows at least partly in view.
    pub(crate) fn visible_rows(&self) -> std::ops::Range<usize> {
        let first = (self.scroll / self.pitch) as usize;
        let end = ((self.scroll + self.rect.bottom() - self.top) / self.pitch).ceil() as usize;
        first.min(self.rows)..end.min(self.rows)
    }

    /// The `+` add-drum button: `ADD_D` square, `ADD_GAP` below the last row, centred in the header cell.
    pub(crate) fn add_button(&self) -> Rect {
        let d = self.u * ADD_D;
        let top = self.row_top() + self.rows as f32 * self.pitch + self.u * ADD_GAP;
        let left = self.rect.left() + (self.header_w - d) / 2.0;
        self.snap_rect(Rect::from_min_size(pos2(left, top), vec2(d, d)))
    }

    /// Everything right of the headers and below the strip.
    pub(crate) fn grid(&self) -> Rect {
        Rect::from_min_max(pos2(self.grid_left(), self.top), self.rect.max)
    }

    /// `n` pitches from `start`, snapped, less one device pixel at the far end.
    pub(crate) fn span(&self, start: f32, pitch: f32, n: usize) -> Rangef {
        let px = 1.0 / self.ppp;
        Rangef::new(
            self.snap(start + n as f32 * pitch),
            self.snap(start + (n + 1) as f32 * pitch) - px,
        )
    }

    pub(crate) fn header(&self, row: usize) -> Rect {
        Rect::from_x_y_ranges(
            self.rect.left()..=self.header_right(),
            self.span(self.row_top(), self.pitch, row),
        )
    }

    /// Step `step`'s cell in row `row`: x0 = x0 + step·step_w, x1 = x0 + step_w − 1 px; y0 = row top, y1 = row top +
    /// pitch − 1 px.
    fn cell(&self, row: usize, step: usize) -> Rect {
        Rect::from_x_y_ranges(
            self.span(self.x0, self.step_w, step),
            self.span(self.row_top(), self.pitch, row),
        )
    }

    /// The (row, step) under `p`, or nearest to it.
    fn cell_at(&self, p: Pos2) -> (usize, usize) {
        let row = ((p.y - self.row_top()) / self.pitch).max(0.0) as usize;
        let step = ((p.x - self.x0) / self.step_w).max(0.0) as usize;
        (row.min(self.rows.saturating_sub(1)), step)
    }

    /// The (row, step) under `p`: anywhere on the grid's rows from content beat 0, past the loop too.
    fn hit(&self, p: Pos2) -> Option<(usize, usize)> {
        if !self.grid().contains(p) || p.x < self.x0 {
            return None;
        }
        let row = ((p.y - self.row_top()) / self.pitch) as usize;
        let step = ((p.x - self.x0) / self.step_w) as usize;
        (row < self.rows).then_some((row, step))
    }
}

#[cfg(test)]
mod tests;
