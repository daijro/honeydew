//! The playlist screen: time ruler, tracks, transport bar and keyboard.
//! Bands and keys are laid out by pure functions; the painters below consume their rects.

use std::ops::Range;
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, Id, Painter, PointerButton, Pos2, Rangef, Rect, Sense, Vec2, pos2, vec2,
};

use honeydew_audio::device::{AudioCommand, OutputSnapshot};

use crate::history::Step;
use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::settings::Settings;
use crate::song::{ChannelKind, LaneKind, PPQ, Song, TimeSig};
use crate::theme::{RECORD_COLOR, channel_color};
use crate::ui::add_channel_menu::add_channel_menu;
use crate::ui::buttons::{
    bubble_button, captioned_button, filled_circle_button, outline_circle_button, tab_button,
};
use crate::ui::clips::{Clips, LiveTake, TimeAxis, grid_ticks, label_font, song_end_beats};
use crate::ui::ctrl_menu::Touch;
use crate::ui::drum_grid::edit::{MARK_GAP, marks};
use crate::ui::focus;
use crate::ui::keyboard_settings::{
    KeyboardMode, PANEL_H, keys_rect, knob_rects, panel_target, panel_tick, piano_rect, strip_rect,
    wheel_rects,
};
use crate::ui::mini_mixer::{self, RowKind};
use crate::ui::panel_resize::{self, PanelGrab, Sizes, Zone};
use crate::ui::piano_roll::notes::{Snap, SnapMode};
use crate::ui::preview::ClipKey;
use crate::ui::region::{self, TimeRange};
use crate::ui::sample_row;
use crate::ui::scroller::{self, Axis, Scroller, X, Y, Zones};
use crate::ui::tempo::transport_bpm_text;
use crate::ui::timeline::{self, KnobGesture, PlayheadKnob, RULER_H, RangeRuler, RangeSelect};
use crate::ui::track_menu::{TrackOp, track_menu};
use crate::ui::transport_extras::{
    BLINK_FROM, Load, Meter, MeterLabels, RamReadout, SystemMemory, cpu_blink,
};
use crate::ui::views::{AppCtx, MenuTab, Nav, Overlay, View};
use crate::ui::widgets;
use crate::ui::{UiCtx, chord_editor, chord_keyboard, gesture, keyboard, popup, wheels};

// Sizes in U unless noted.
/// The transport bar's height, in U: the bottom panel's button bar, above the keyboard settings strip.
pub(crate) const TRANSPORT_H: f32 = 1.75;
const HEADER_W: f32 = 1.5;
/// How much wider the expanded header column is (13.5 U in all), and its per-frame law c ← c + 0.4·(t − c), snapping
/// to t within 0.05 (the drum mixer column's law).
const HEADER_GROW: f32 = 12.0;
const COLUMN_RATE: f32 = 0.4;
const COLUMN_SNAP: f32 = 0.05;
const ROW_H: f32 = 2.0;
/// The rows' zoom range, in U (zoom y in [0.24/U, 0.5/U] rows a pixel).
const ROW_RANGE: [f32; 2] = [2.0, 100.0 / 24.0];
/// One beat of the grid at zoom 0 (the reference of [`MainScreen::zoom`]).
const BEAT_W: f32 = 1.2;
/// The song name's rect starts this far into the ruler row (U).
const SONG_NAME_X: f32 = 2.0;
/// The rows' scrollbar: its thickness and the band across it that takes a drag. A thin bar on the track area's right
/// edge, as the browser's, grabbable within half a U.
const BAR_W: f32 = 0.15;
const BAR_HIT: f32 = 0.5;
/// App-logo quarter-disc radius (centred on the top-right corner) and its icon's distance from both edges; the
/// show-keyboard `↖` on the bottom-right corner is drawn the same way.
const LOGO_R: f32 = 2.0;
const LOGO_ICON_INSET: f32 = 0.85;
/// Right-edge tab half-disc radius.
const EDGE_TAB_R: f32 = 1.0;
/// The add-channel `+`: the disc diameter, and the length of each bar of the plus inside it, tip to tip.
const ADD_D: f32 = 2.0;
const PLUS_LEN: f32 = 1.12;
/// The gap between the last row and the `+`.
const ADD_GAP: f32 = 0.75;
/// Room for the `+` below the last row: the gap, the circle, and the gap again below it.
const ADD_ROOM: f32 = 2.0 * ADD_GAP + ADD_D;
/// Round transport buttons: mixer, keyboard settings, play and close.
const ROUND_BUTTON_D: f32 = 1.25;
/// The `Preview` toggle's circle: wide enough for its 6 pt label, as the Modules browser's `Search` (1.5 U).
const PREVIEW_D: f32 = 1.5;
/// One slot of the centre transport group and of the CPU / RAM readouts.
const SLOT_W: f32 = 1.75;
/// The CTRL button's slot in the centre group.
const CTRL_SLOT: f32 = 4.0;
/// Centres of a readout's two lines, from the top of the transport bar.
pub const READOUT_LINES: [f32; 2] = [0.65, 1.2];
/// CPU bar width and height.
const CPU_BAR: [f32; 2] = [0.6, 0.12];
/// The armed multitrack channel's dot in its header: its radius, and its centre's offset from the header's top-left
/// corner on both axes.
const ARMED_DOT_R: f32 = 0.1;
const ARMED_DOT_AT: f32 = 0.3;
/// Pause glyph: bar width (also the gap between the bars) and height.
const PAUSE_BAR: [f32; 2] = [0.15, 0.5];
/// Icon size factors.
const TRANSPORT_ICON: f32 = 0.7;
const TAB_ICON: f32 = 0.6;

/// The CPU meter bar colour (a literal, not a theme role).
const CPU_GREEN: Color32 = Color32::from_rgb(0x10, 0xF3, 0x10);

/// One track row of the playlist: a channel's track (lane). Built every frame, so it copies no name.
#[derive(Clone, Copy)]
pub struct TrackRow {
    pub channel: usize,
    /// The lane in the channel; 0 is the primary track.
    pub lane: usize,
    pub icon: IconId,
    /// Channel hue, see [`channel_color`].
    pub hue: f32,
    /// A primary track of an Audio Record channel whose rack arm button is on (CHHD@1060): with multitrack recording
    /// on, its header shows the armed dot.
    pub armed: bool,
}

/// The playlist's rows, top to bottom: each channel's ([`Song::playlist`] order) primary track, then its sub-tracks
/// unless they are hidden (`Hide tracks`). A primary header shows the channel kind's icon, a sub-track's its own
/// kind's.
pub fn track_rows(song: &Song) -> Vec<TrackRow> {
    let mut rows = Vec::new();
    for channel in song.playlist() {
        let ch = &song.channels[channel];
        let shown = if ch.zipped { 1 } else { ch.lanes.len() };
        for (lane, l) in ch.lanes.iter().enumerate().take(shown) {
            rows.push(TrackRow {
                channel,
                lane,
                icon: if lane == 0 {
                    kind_icon(ch.kind)
                } else {
                    lane_icon(l.kind)
                },
                hue: ch.hue,
                armed: lane == 0 && ch.kind == ChannelKind::AudioRecord && ch.file.record_armed,
            });
        }
    }
    rows
}

/// One frame of a track header's drag: the finger puts channel `dragged`'s primary row at row `raw` (fractional) of
/// `order`, the channels as the rows show them (channel `c` is `counts[c]` rows tall). The target row is `raw` clamped
/// to [0, last row] (the last row less the channel's extra tracks as they show) and rounded; the channel there swaps
/// places with the dragged one. Returns the row to draw the dragged primary at: `raw`, at 0.05 of the finger past
/// either end (the rubber band). Past one-row channels, the channel that held the target row is the one whose swap
/// lands the dragged primary nearest the target, and strictly nearer than it is: a one-row channel is the target row's
/// own, and no swap undoes itself under a still finger.
fn drag_step(order: &mut [usize], counts: &[usize], dragged: usize, raw: f32) -> f32 {
    let last = (counts.iter().sum::<usize>() - counts[dragged]) as f32;
    let target = raw.clamp(0.0, last).round() as usize;
    let starts: Vec<usize> = (order.iter())
        .scan(0, |row, &c| Some(std::mem::replace(row, *row + counts[c])))
        .collect();
    if let Some(d) = order.iter().position(|&c| c == dragged) {
        // Where the dragged primary lands, swapped with the channel at place `x`.
        let lands = |x: usize| match x <= d {
            true => starts[x],
            false => starts[x] + counts[order[x]] - counts[dragged],
        };
        let miss = |x: usize| lands(x).abs_diff(target);
        if let Some(x) = (0..order.len())
            .min_by_key(|&x| miss(x))
            .filter(|&x| miss(x) < miss(d))
        {
            order.swap(d, x);
        }
    }
    match raw {
        r if r < 0.0 => r * 0.05,
        r if r > last => last + (r - last) * 0.05,
        r => r,
    }
}

/// A sub-track header's icon: automation takes the master's (whose primary is its Level automation track).
fn lane_icon(kind: LaneKind) -> IconId {
    match kind {
        LaneKind::Notes => IconId::Piano,
        LaneKind::Drums => IconId::StepSeq,
        LaneKind::Audio => IconId::AudioClip,
        LaneKind::Automation => IconId::Browser,
    }
}

/// The icon in a channel's primary track header and mixer strip.
pub fn kind_icon(kind: ChannelKind) -> IconId {
    match kind {
        ChannelKind::Master => IconId::Browser,
        ChannelKind::Drums => IconId::StepSeq,
        ChannelKind::Instrument | ChannelKind::Rack | ChannelKind::Synth(_) => IconId::Piano,
        ChannelKind::AudioClip => IconId::AudioClip,
        ChannelKind::AudioRecord => IconId::AudioRecord,
        ChannelKind::EffectTrack => IconId::Routing,
    }
}

/// Vertical extents of the four bands, in points from the top of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    pub ruler: Rangef,
    pub tracks: Rangef,
    pub transport: Rangef,
    pub keyboard: Rangef,
}

/// Splits a screen `screen_h` points tall into the four bands for a UI unit of `u` points and a keyboard panel (the
/// transport bar over the keys) `panel_h` points tall, at the screen's bottom (the panel's height is its preferred
/// height, animated).
pub fn band_layout(screen_h: f32, u: f32, panel_h: f32) -> Bands {
    let transport = Rangef::new(screen_h - panel_h, screen_h - panel_h + u * TRANSPORT_H);
    let keyboard = Rangef::new(transport.max, screen_h);
    let ruler = Rangef::new(0.0, u * RULER_H);
    Bands {
        ruler,
        tracks: Rangef::new(ruler.max, transport.min.max(ruler.max)),
        transport,
        keyboard,
    }
}

/// What the user did on the playlist screen this frame. Play / Stop shows in [`MainScreen::playing`].
#[derive(Default)]
pub struct ScreenActions {
    /// The channel kind picked in the add-channel popup.
    pub add: Option<ChannelKind>,
    /// The channel whose track header was clicked.
    pub header: Option<usize>,
    /// The track menu's pick, for track `.1` of channel `.0` (`Insert Channel` shows the add-channel popup instead,
    /// see [`Self::insert`]).
    pub track: Option<(usize, usize, TrackOp)>,
    /// `Insert Channel`'s pick: the channel kind, to go in at the index.
    pub insert: Option<(usize, ChannelKind)>,
    /// A held track header's drag dropped: the channels' new playlist order, their indices top to bottom
    /// ([`Song::set_playlist`]). The rows show the drag live meanwhile.
    pub reorder: Option<Vec<usize>>,
    /// The keyboard keys held, and those tapped (pressed and lifted) within this frame, which sound for a short gate.
    pub keys: keyboard::Played,
    /// REV was clicked.
    pub rewind: bool,
    /// A transport button, the logo corner, the rack tab, a track header (its rack) or a clip's `Edit` (its
    /// editor).
    pub nav: Option<Nav>,
    /// An editor's `Undo` or `Redo` was clicked.
    pub step: Option<Step>,
    /// An editor's `Preview` was clicked.
    pub preview: bool,
    /// A ruler's tap this frame, in any view ([`AppCtx::set_playhead`]): the playhead's song tick.
    pub playhead: Option<u64>,
    /// A playhead knob's drag let go, the playlist's or an editor's ([`AppCtx::seek`]): seek to this song tick, playing
    /// or stopped.
    pub seek: Option<u64>,
    /// The sample row's action, for its clip ([`ScreenInput::sample_row`]).
    pub sample_row: Option<sample_row::RowAction>,
    /// The keyboard panel's new sizes, in points: a hand resize let go, or the tap toggle ([`panel_resize`]).
    pub panel_sizes: Option<Sizes>,
    /// The pitch and mod wheels' moves and press, for the current channel ([`wheels::wheels`]).
    pub wheels: wheels::Wheels,
    /// The chord keyboard's note-ons and note-offs, in order, for the current channel's live input.
    pub chord_notes: Vec<chord_keyboard::Note>,
    /// The chord editor's edit: strip `.0`'s chord is now `.1` (in the set in use, transposed to the key).
    pub chord_edit: Option<(usize, crate::song::chords::Chord)>,
}

/// What the playlist screen draws from, besides its own state.
#[derive(Clone, Copy)]
pub struct ScreenInput<'a> {
    /// The view shown: the chrome shows `Undo`, `Lock`, `Preview` and the drum-view controls by it.
    pub view: View,
    /// The rows, from [`track_rows`]: `tracks[0]` is the master track.
    pub tracks: &'a [TrackRow],
    /// The channel the keyboard plays: its colour marks the root keys and fills the pressed ones, and the rack tab
    /// opens its rack.
    pub selected: usize,
    /// The keyboard's labels and the CPU / RAM readouts follow them.
    pub settings: &'a Settings,
    /// This frame's output snapshot, if the audio runs: it places the playhead while playing and fills the
    /// CPU bar.
    pub audio: Option<&'a OutputSnapshot>,
    /// The song's tempo, for the BPM button.
    pub bpm: f64,
    /// There is something to undo, and something to redo: the editors' `Undo` and `Redo`.
    pub history: (bool, bool),
    /// The clip Preview plays alone, while it is on: the editors' `Preview` is lit, and the playlist dims every other
    /// clip.
    pub preview: Option<ClipKey>,
    /// The editor shown edits a clip ([`crate::ui::preview::current`]): it shows `Preview`.
    pub can_preview: bool,
    /// The sample row's values while it replaces the keyboard ([`sample_row::current`]: the selected channel's main
    /// lane is audio and a clip is current).
    pub sample_row: Option<sample_row::Row>,
    /// The current channel's hex pads replace the keyboard ([`crate::ui::views::drum_editor::pads_channel`]: its first
    /// lane is a drum lane), in every view; the app draws them after the chrome.
    pub pads: bool,
    /// How touches play the keyboard.
    pub keyboard: keyboard::KeyInput,
    /// The keys are a piano (`Classic`, `Scale`, or an automation lane's piano), which may show a second row; false
    /// for the chord kinds, whose keyboard the piano stands in for in one row.
    pub piano: bool,
    /// An overlay lies over the pointer ([`crate::ui::views::covered`]): no resize grab or wheel zoom starts there.
    pub covered: bool,
    /// The current channel's MOD (CC 1) and PITCH values, which the wheels show.
    pub wheels: [f32; 2],
    /// The chord keyboard replaces the piano (kinds `Chords` and `Strum` on a notes channel), playing this.
    pub chords: Option<chord_keyboard::ChordInput<'a>>,
}

/// Where views and overlays go this frame: the bands of [`band_layout`] across the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Regions {
    pub screen: Rect,
    /// The content area, above the transport bar: the view and the rack share it.
    pub area: Rect,
    /// The view shown (the playlist's ruler and tracks, an editor or the mixer): the area left of the rack, at least
    /// 1 U wide, so it is resized, not covered, as the rack slides in ([`Self::with_rack`]).
    pub editor: Rect,
    /// The rack at its slide ([`crate::ui::rack::rack_rect`]): just past the area's right edge while closed.
    pub rack: Rect,
    pub transport: Rect,
    /// The keys' area (the pads', the sample row's): the keyboard band, less the settings strip while that is open
    /// ([`Self::with_strip`]).
    pub keyboard: Rect,
    /// The keyboard settings strip, under the transport bar: 3 U tall while open, none when closed.
    pub strip: Rect,
    /// The `•••` CTRL button, for anchoring its menu.
    pub ctrl: Rect,
}

impl Regions {
    /// The regions of `screen` with the keyboard panel at its default height, [`PANEL_H`] U.
    pub fn new(screen: Rect, m: &Metrics) -> Self {
        Self::sized(screen, panel_target(PANEL_H, screen.height(), m), m)
    }

    /// The regions of `screen` with the keyboard panel `panel_h` points tall ([`panel_target`], [`band_layout`]).
    pub fn sized(screen: Rect, panel_h: f32, m: &Metrics) -> Self {
        let bands = band_layout(screen.height(), m.u(1.0), panel_h);
        let band = |y: Rangef| {
            Rect::from_x_y_ranges(
                screen.x_range(),
                screen.top() + y.min..=screen.top() + y.max,
            )
        };
        let transport = band(bands.transport);
        let keyboard = band(bands.keyboard);
        let area = band(Rangef::new(bands.ruler.min, bands.tracks.max));
        let panel = Rect::from_min_max(transport.min, keyboard.max);
        Self {
            screen,
            area,
            editor: area,
            rack: crate::ui::rack::rack_rect(area, 0.0, m),
            transport,
            keyboard,
            strip: strip_rect(panel, false, m),
            ctrl: transport_slot(transport, CTRL_SLOT, m),
        }
    }

    /// The regions with the keyboard settings strip `open` or closed: it sits in the bottom panel under the transport
    /// bar and the keys start below it.
    pub fn with_strip(self, open: bool, m: &Metrics) -> Self {
        let panel = Rect::from_min_max(self.transport.min, self.keyboard.max);
        Self {
            keyboard: keys_rect(panel, open, m),
            strip: strip_rect(panel, open, m),
            ..self
        }
    }

    /// The regions while the keyboard panel is hidden: the editor area reaches the bottom of the screen, and the
    /// transport bar and the keyboard lie below it, off screen (what is drawn there is clipped and takes no input).
    pub fn without_panel(self) -> Self {
        let down = Vec2::new(0.0, self.screen.bottom() - self.transport.top());
        Self {
            area: self.area.with_max_y(self.screen.bottom()),
            editor: self.editor.with_max_y(self.screen.bottom()),
            rack: self.rack.with_max_y(self.screen.bottom()),
            transport: self.transport.translate(down),
            keyboard: self.keyboard.translate(down),
            strip: self.strip.translate(down),
            ctrl: self.ctrl.translate(down),
            ..self
        }
    }

    /// The regions with the rack slid in by `open` (0 closed … 1 open): the view's width is max(rack left − area left,
    /// 1 U), so the rack only overlaps it below 1 U.
    pub fn with_rack(self, open: f32, m: &Metrics) -> Self {
        let rack = crate::ui::rack::rack_rect(self.area, open, m);
        let right = rack.left().max(self.area.left() + m.u(1.0));
        Self {
            editor: self.area.with_max_x(right),
            rack,
            ..self
        }
    }
}

/// Transient state of the playlist screen.
#[derive(Default)]
pub struct MainScreen {
    /// Toggled by the play button; the app sends `Play` / `Stop` when it changes.
    pub playing: bool,
    /// The keyboard panel's height in points as last drawn, on its way to the preferred height ([`Self::panel_h`]);
    /// `None` before the first frame.
    pub(crate) panel: Option<f32>,
    /// The hand resize, tap toggle and scroll zone over the bar and the strip, with the keys' fling.
    pub(crate) grab: PanelGrab,
    /// Where the grab may start: the last drawn bar and strip (none while the panel is hidden).
    zone: Option<Zone>,
    /// This frame's grab, before the layout ([`Self::grab`]): its scroll and zoom move the keys.
    grabbed: panel_resize::Grabbed,
    /// The piano's scroll and zoom, which the scroll zone moves; both rows share it.
    pub(crate) keys_view: keyboard::KeyView,
    /// Initial centering waits until the piano has a visible viewport.
    keys_initialized: bool,
    /// Each channel's last piano view, by channel id, and the channel whose view `keys_view` is ([`Self::keys_for`]).
    keys_views: std::collections::HashMap<i32, keyboard::KeyView>,
    keys_owner: Option<i32>,
    /// The chord keyboard's fingers, voices and highlights.
    pub(crate) chords: chord_keyboard::ChordKeyboard,
    /// PITCH's last move left it off 0.5: a frame without the wheels springs it back ([`Self::pitch_back`]).
    pitch_bent: bool,
    /// REC: toggled by its button; the app disarms it at any stop.
    pub armed: bool,
    /// The play-start point, a song tick: where Play last started or the last seek went; REV goes back to it.
    pub play_start: u64,
    /// The takes being recorded, which the playlist draws as live bars; the app sets them every frame.
    pub takes: Vec<LiveTake>,
    /// The add-channel popup is open; it is centred on the `+`.
    add_channel_open: bool,
    /// The track menu is open for track `.1` of channel `.0`, centred on the point.
    pub(crate) track_menu: Option<(usize, usize, Pos2)>,
    /// `Insert Channel`'s add-channel popup is open: the index the pick goes in at, and the popup's centre.
    pub(crate) insert: Option<(usize, Pos2)>,
    /// How far the track rows are scrolled up, in points (0 = the master row is at the top).
    pub(crate) scroll_y: f32,
    /// How far the timeline is scrolled: the beat at the grid's left edge.
    pub(crate) scroll_x: f64,
    /// Horizontal zoom: a beat is [`BEAT_W`] times two to this power.
    pub(crate) zoom: f32,
    /// Vertical zoom: a track row's height in U ([`ROW_RANGE`]). `None` until the first frame of a song, which takes
    /// the song's stored view (HEAD@296 zoom y, rows a pixel) through the same clamp.
    pub(crate) row_u: Option<f32>,
    /// The scroll and zoom controller.
    scroller: Scroller,
    /// The selected clip and the gesture on it.
    pub clips: Clips,
    /// The time range on the ruler, in song ticks.
    pub(crate) region: RangeSelect,
    knob: PlayheadKnob,
    ram: RamReadout,
    /// The `CPU` and `RAM` labels' presses and tooltips.
    meters: MeterLabels,
    /// The shared performance data's load, peak and smoothed load, for the `CPU` meter and tooltip; the app sets them
    /// every frame.
    pub meter: Load,
    /// The `CPU` or `RAM` label was tapped: the app opens PROJECT scrolled to the Performance block, once.
    pub performance_tap: bool,
    /// The keyboard panel (transport bar and keyboard) is hidden: the transport's close X hid it, and the `↖` shows it
    /// again ([`Self::show_keyboard_button`]).
    pub panel_hidden: bool,
    /// The header column's expansion c (0: 1.5 U, 1: 13.5 U) and its target, which a tap on the current track's header
    /// toggles.
    column: (f32, f32),
    /// A header press is held past its long press: the touch is the header's, and the scroller's no more. (Undecided,
    /// it is the scroller's too: their calm zones are the same 0.5 U up or down.)
    header_busy: bool,
    /// The hold on a header, until its release.
    hold: Option<HeaderHold>,
    /// The ruler's Snap button: what the playhead knob's drop and the ruler's taps and range snap to.
    pub(crate) snap: Snap,
    /// The snap-mode list is open.
    snap_menu: bool,
    /// A mini-mixer control's touch (its channel and param), for the app's recent control; the app takes it.
    pub touch: Option<(usize, Touch)>,
    /// The playlist / clip-editor split: the app lays an open clip editor out under the playlist by it.
    pub(crate) split: crate::ui::split::Split,
}

/// A track header held past its 300 ms.
#[derive(Clone, Debug)]
struct HeaderHold {
    /// The track row it went down on and where in rows (fractional), the touch-down point and the scroll then.
    row: usize,
    start: f32,
    origin: Pos2,
    scroll: f32,
    /// It moved off the calm zone vertically after the hold: a reorder.
    dragging: bool,
    /// While it drags: the channels in the order the rows show ([`drag_step`]), and the row the dragged primary shows
    /// at (fractional).
    live: Option<(Vec<usize>, f32)>,
}

impl MainScreen {
    /// The transport `RAM` readout's last value (`NNN MB`; `--` where the platform does not report it; empty before the
    /// first read).
    pub fn ram_text(&self) -> &str {
        self.ram.text()
    }

    /// The machine's memory at the readout's last read, where the platform reports it.
    pub fn system_memory(&self) -> Option<SystemMemory> {
        self.ram.system()
    }

    /// The playlist view in `app.regions.editor`: the time ruler, the track rows with their clips, the `+` with
    /// its popup and the playhead. The track headers' gestures are [`Self::header_gestures`]; the clips are edited
    /// in place (see [`Clips`]), and a clip's `Edit` opens its editor.
    pub fn playlist(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        input: ScreenInput,
        actions: &mut ScreenActions,
    ) {
        let cx = app.cx;
        let UiCtx { theme, m } = cx;
        let ScreenInput {
            tracks,
            audio,
            settings,
            ..
        } = input;
        let c = &theme.colors;
        let text = c.text.to_color32();
        let editor = app.regions.editor;
        let (ruler_area, track_area) = editor.split_top_bottom_at_y(editor.top() + m.u(RULER_H));
        self.column.0 = column_step(self.column.0, self.column.1);
        if self.column.0 != self.column.1 {
            ui.ctx().request_repaint();
        }
        // The header column, then a t gap: the time ruler and the grid start after it.
        let column = track_area.with_max_x(track_area.left() + self.header_w(m));
        let grid = track_area.with_min_x(column.right() + m.t());
        let expanded = self.column.0 == 1.0;
        // A song with no clips counts one bar, so that it neither opens at the closest zoom nor stops scrolling.
        // Any clip's end is its own.
        let end = song_end_beats(app.song);
        let content = Content {
            rows: tracks.len(),
            end: Some(end)
                .filter(|&e| e > 0.0)
                .unwrap_or(f64::from(app.song.time_sig.bar_beats())),
            d: app.song.time_sig.subdivision,
        };
        if self.row_u.is_none() {
            self.restore_view(&mut app.song.file.playlist_view, track_area, content, m);
        }

        // Scrolling first: the rows, the clips and the `+` are placed from the new offsets. The view follows the
        // playhead while the song plays with Settings `Continuous scrolling during playback` on.
        let before = self.view();
        let playing = self.playing && settings.continuous_scroll;
        let lead = app.audio.playhead_lead();
        let follow =
            (audio.filter(|_| playing)).map(|s| (s.transport_tick + lead) as f64 / f64::from(PPQ));
        let scroll = self.scroll(
            ui,
            ruler_area,
            track_area,
            content,
            (follow, !app.covered),
            m,
        );
        let row_h = self.row_h(m);
        let axis = self.axis(grid.left(), m);
        // The ruler row is a `background` panel over the full width; the time ruler starts at the grid.
        let marks = ruler_area.with_min_x(grid.left());
        ui.painter()
            .rect_filled(ruler_area, 0.0, c.background.to_color32());
        timeline::ruler(
            ui.painter(),
            marks,
            marks,
            axis.origin,
            axis.beat_w,
            app.song.time_sig,
            &cx,
        );
        let rows = visible_rows(scroll, track_area.height(), row_h, tracks.len());
        // A hold ends with its press, wherever that was.
        if ui.input(|i| !i.pointer.any_down() && !i.pointer.any_released()) {
            self.hold = None;
        }
        let placed = self.placed(tracks);
        let visible = |at: f32| at + 1.0 > rows.start as f32 && at < rows.end as f32;
        let top_of = |at: f32| track_area.top() - scroll + at * row_h;
        let shown: Vec<(TrackRow, f32)> = (placed.iter())
            .filter(|p| visible(p.1))
            .map(|&(i, at)| (tracks[i], top_of(at)))
            .collect();
        let track_painter = ui.painter_at(track_area);
        // The current track's row is lit, once there are two channels.
        let current = (app.song.channels.len() >= 2).then(|| self.clips.current_track(app));
        paint_track_rows(
            &track_painter,
            track_area,
            grid.left(),
            &shown,
            axis,
            (app.song.time_sig, row_h, current),
            &cx,
        );
        // Headers sense clicks only, so a drag starting on one still scrolls once it leaves the calm zone. A header keeps
        // its track row's id wherever it shows, so a hold follows its track; the held one is sensed off screen too.
        let mut lanes = Vec::new();
        self.header_busy = false;
        for &(i, at) in &placed {
            let held = self.hold.as_ref().is_some_and(|h| h.row == i);
            if !visible(at) && !held {
                continue;
            }
            let TrackRow { channel, lane, .. } = tracks[i];
            let top = top_of(at);
            let row = Rect::from_x_y_ranges(track_area.x_range(), top..=top + row_h);
            let lane_rect = row.with_min_x(grid.left());
            // At c = 1 only the header's own part takes its gestures; collapsed or animating, the whole row does.
            let header = if expanded {
                let cells = row
                    .intersect(column)
                    .with_max_y(top + row_h - 1.0 / m.ppp());
                mini_mixer::header_rect(RowKind::of(app.song, &tracks[i]), cells, true, m)
            } else {
                row.intersect(column)
            };
            let mut response = ui.interact(
                header.intersect(track_area),
                Id::new(("track_header", i)),
                Sense::click(),
            );
            let track = (i, channel, lane, top);
            self.header_gestures(ui, app, &mut response, track, scroll, tracks, actions);
            if !visible(at) {
                continue;
            }
            // Clips leave the 1-px gap under the row clear.
            lanes.push((
                channel,
                lane,
                lane_rect.with_max_y(lane_rect.bottom() - 1.0 / m.ppp()),
            ));
        }
        let (preview, takes) = (input.preview, &self.takes);
        self.clips.left_calm = self.scroller.left_calm();
        let snap = self.snap;
        if let Some(nav) = self
            .clips
            .show(ui, app, grid, &lanes, axis, preview, takes, snap)
        {
            actions.nav = Some(nav);
        }
        // Children get the touch first: a held header, a marquee (or the hold that starts one) takes it from the
        // scroller; a ruler press waiting for its long press leaves it the scroller's until a range starts.
        if self.header_busy || self.clips.busy(ui.input(|i| i.time)) {
            self.scroller.yield_touch();
        }
        let snap = self
            .snap
            .active(grid_ticks(axis, app.song.time_sig, m), app.song.time_sig);
        let ruler_at = self.region(ui, app, marks, axis, snap);
        if self.region.pending() {
            self.scroller.claim();
        } else if self.region.dragging() {
            self.scroller.yield_touch();
        }
        // The rows' scrollbar, over the clips on the track area's right edge.
        let mut axes = self.axes(track_area, content, m);
        let was = axes;
        let bar = track_area.with_min_x(track_area.right() - m.u(BAR_W));
        let id = Id::new("track_scrollbar");
        scroller::scrollbar(ui, id, bar, m.u(BAR_HIT), &mut axes[Y], text);
        self.take(axes, was, m);
        // The transport's playhead, playing or stopped (a tap on the ruler sets it; tick 0 without an output), and its
        // line, which shows whatever the play state. While the knob is dragged the line and the disc follow the
        // finger: its beat clamped to [view-left beat, song end] and rounded to the Snap unit; the drop seeks there,
        // and the playhead stays on it until the output reports a tick in that beat, or for a quarter second
        // (`PlayheadKnob::line_x`).
        let tick = audio.map(|s| s.transport_tick);
        let end = end * f64::from(PPQ);
        let ruler_at = RangeRuler {
            playhead: tick.map(|t| t as f64),
            ..ruler_at
        };
        // Drawn at the smooth playhead (`AudioLink::playhead_lead`), never used for an edit.
        let drawn = tick.map_or(0, |t| t + lead);
        let x = (self.knob).line_x(ui, m, &ruler_at, end, axis.x(drawn as f64));
        let playhead = Some(x).filter(|&x| grid.x_range().contains(x));
        // Over the clips and under their selection chrome (the slot they left), painted from the column's right edge:
        // the column covers only what lies left of it, so at beat 0 the line's left half shows in the t gap.
        let line_painter = ui.painter_at(track_area.with_min_x(column.right()));
        let line = timeline::playhead_line_shapes(grid, x, &cx);
        match self.clips.line_slot.take() {
            Some(slot) => line_painter.set(slot, line),
            None => line_painter.extend(line),
        }

        // The header column over it (the mini mixer at c = 1), the current channel's rows last.
        let col = mini_mixer::Column {
            left: column.left(),
            width: column.width(),
            expanded,
            current: self.clips.current_track(app),
            press: self.cell_press(ui, row_h * m.ppp() - 1.0),
            multitrack: settings.multitrack_recording,
            meters: audio
                .filter(|s| !s.stream_failed)
                .map(|s| &s.module_meters[..]),
        };
        let out = mini_mixer::show(ui, &cx, app.song, &shown, row_h, &col, column);
        if !out.changes.is_empty() {
            app.channel_param_edited();
        }
        for (channel, target, value) in out.changes {
            app.audio.send(AudioCommand::SetParam {
                channel,
                instance_id: target.instance_id,
                index: target.param,
                value,
            });
        }
        self.touch = out.touch.or(self.touch);
        self.ruler_corner(ui, app, ruler_area, column, expanded);

        // The `+` sits below the last row and scrolls with the rows, so it is clipped to the track area.
        let add_center = add_button_center(
            app.regions.screen,
            track_area,
            tracks.len(),
            row_top(track_area, scroll, tracks.len(), row_h),
            m,
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(track_area));
        let add = filled_circle_button(
            ui,
            Id::new("add_channel_button"),
            add_center,
            m.u(ADD_D),
            text,
        );
        for bar in plus_bars(add_center, m) {
            ui.painter().rect_filled(bar, 0.0, c.menu_fill.to_color32());
        }
        ui.set_clip_rect(outer_clip);
        // While the popup is open its backdrop takes every click, so this is only ever a click that opens it.
        self.add_channel_open |= add.clicked();
        let screen = ui.max_rect();
        actions.add = self.add_channel_popup(ui.ctx(), add_center, screen, &cx);
        actions.track = self.track_menu(ui.ctx(), app.song, screen, &cx);
        actions.insert = self.insert_popup(ui.ctx(), screen, &cx);

        // Before the knob, which covers it at beat 0 and takes the touches there while it shows.
        self.snap_input(ui, ruler_area, &cx);
        // The playhead knob rides the live playhead on the time ruler's middle line (`timeline::PlayheadKnob`), fading
        // in while the playhead is on the grid and the drum editor's rule holds ([`timeline::knob_shown`]), out
        // otherwise. A tap plays; a drag shows and seeks to the snapped beat above.
        let knob = Id::new("playhead_knob");
        self.knob.armed = self.armed;
        let shown =
            playhead.is_some() && timeline::knob_shown(self.playing, self.clips.row.showing());
        match (self.knob).show(ui, knob, marks, x, shown, &cx) {
            Some(KnobGesture::Tap) => self.playing = true,
            Some(KnobGesture::Drop(x)) => {
                actions.seek = Some(self.knob.drop_tick(ui, &ruler_at, end, x));
            }
            None => {}
        }
        // The keymap's editing actions, after the popups (the snap menu's too), which take Escape first
        // (`ui/edit_keys.rs`); paste and Add clip land on the snap mode's grid, as `+Clip` does.
        let auto = grid_ticks(axis, app.song.time_sig, m);
        self.clips
            .keys(ui, app, self.snap.grid(auto, app.song.time_sig));
        // A view that moved goes into the song, saved with it (HEAD@296; no edit, nor an unsaved change).
        if before != self.view() {
            let (beat_w, row_h) = (self.axis(0.0, m).beat_w, self.row_h(m));
            app.song.file.playlist_view = [
                self.scroll_x,
                f64::from(self.scroll_y / row_h),
                f64::from(1.0 / (beat_w * m.ppp())),
                f64::from(1.0 / (row_h * m.ppp())),
            ];
        }
    }

    /// One frame of the current header cell's press feedback: the cell's vertical scale. While a header is held past
    /// its 300 ms the knob law grows it (a ← 0.6·a), and let go it wobbles back ([`widgets::pulse`]); `h` is the cell's
    /// height in device pixels.
    fn cell_press(&self, ui: &egui::Ui, h: f32) -> f32 {
        widgets::pulse(ui, Id::new("header_cell"), self.hold.is_some(), h)
    }

    /// The ruler row's top-left cell, over the `background` panel:
    /// - the Snap button, always: rect (x, top, 1.5 U, ruler h − 1 px), the editors' shared button
    ///   ([`widgets::paint_snap_button`]; its touches are [`Self::snap_input`]'s). A tap toggles Snap; a long press (or
    ///   a secondary click) opens the snap-mode list;
    /// - at c = 1 the song's name (its file name, `New Song (n)` untitled) in 12 pt `text`, centred in (x + 2 U, top,
    ///   column width − 2 U, ruler h), middle-truncated keeping 5 characters.
    fn ruler_corner(&self, ui: &egui::Ui, app: &AppCtx, ruler: Rect, column: Rect, expanded: bool) {
        let cx = app.cx;
        let (c, m) = (&cx.theme.colors, cx.m);
        if expanded {
            let rect = Rect::from_x_y_ranges(
                column.left() + m.u(SONG_NAME_X)..=column.right(),
                ruler.y_range(),
            );
            // ponytail: an untitled song's `New Song (n)` is looked up in the song folder every expanded frame (a stat per
            // taken n); cache it by title if that ever shows in a profile.
            let name = app.files.proposed_name(app.song);
            let font = m.font_id(FontSize::Pt12);
            let text = c.text.to_color32();
            widgets::truncated(
                ui.painter(),
                rect,
                Align2::CENTER_CENTER,
                &name,
                font,
                text,
                5,
            );
        }
        let rect = snap_rect(ruler, m);
        widgets::paint_snap_button(ui.painter(), rect, self.snap.on, &cx);
    }

    /// The Snap button's touches ([`Self::ruler_corner`] draws it) and its snap-mode list. Sensed before the playhead
    /// knob, whose square covers it at beat 0: the knob, drawn on top, takes the touches there while it shows (the
    /// disc above the magnet); faded out, it takes none and the button has them.
    fn snap_input(&mut self, ui: &mut egui::Ui, ruler: Rect, cx: &UiCtx) {
        let m = cx.m;
        let rect = snap_rect(ruler, m);
        match widgets::snap_touch(ui, Id::new("playlist_snap"), rect, cx) {
            Some(widgets::SnapTouch::Menu) => self.snap_menu = true,
            Some(widgets::SnapTouch::Toggle) => self.snap.on = !self.snap.on,
            None => {}
        }
        if self.snap_menu {
            let items: Vec<popup::PopupItem<SnapMode>> = (SnapMode::ALL.iter().enumerate())
                .map(|(row, &mode)| popup::PopupItem {
                    current: mode == self.snap.mode,
                    ..popup::PopupItem::new(mode, 0, row, mode.label())
                })
                .collect();
            let ctx = ui.ctx();
            let id = Id::new("playlist_snap_menu");
            let result = popup::show(ctx, id, rect.center(), ctx.content_rect(), &items, cx);
            if let Some(mode) = result.selected {
                self.snap = Snap { on: true, mode };
            }
            if result.close {
                self.snap_menu = false;
                ctx.request_repaint();
            }
        }
    }

    /// The gestures of the header of track row `i` (track `lane` of `channel`) of `tracks`, with a vertical-only calm
    /// zone ([`gesture::vertical`]):
    /// - a tap makes a track current (and its channel), and on the current track toggles the header column;
    /// - a hold (300 ms) makes the track current and shows nothing; released without dragging it opens the track's menu
    ///   at the release point; a channel's primary row (any channel's) dragged vertically moves the channel, live by
    ///   [`drag_step`], and its drop is [`ScreenActions::reorder`];
    /// - more than 0.5 U of vertical travel before the hold leaves the touch to the playlist's scroll;
    /// - a secondary click makes the track current and opens its menu at the pointer.
    #[allow(clippy::too_many_arguments)]
    fn header_gestures(
        &mut self,
        ui: &egui::Ui,
        app: &mut AppCtx,
        response: &mut egui::Response,
        (i, channel, lane, top): (usize, usize, usize, f32),
        scroll: f32,
        tracks: &[TrackRow],
        actions: &mut ScreenActions,
    ) {
        let (m, row_h) = (app.cx.m, self.row_h(app.cx.m));
        let p = gesture::press(ui, response, m, gesture::vertical);
        self.header_busy |= p.long || p.held;
        if p.tap {
            if (channel, lane) == self.clips.current_track(app) {
                self.column.1 = 1.0 - self.column.1;
            } else {
                self.make_current(app, channel, lane, actions);
            }
        }
        if p.long {
            self.make_current(app, channel, lane, actions);
            let origin = ui.input(|i| i.pointer.press_origin());
            self.hold = origin.map(|origin| HeaderHold {
                row: i,
                start: i as f32 + (origin.y - top) / row_h,
                origin,
                scroll,
                dragging: false,
                live: None,
            });
        }
        if let Some(hold) = self
            .hold
            .as_mut()
            .filter(|h| h.row == i && (p.held || p.long_released))
        {
            let pos = ui.input(|i| i.pointer.latest_pos()).unwrap_or(hold.origin);
            let dy = pos.y - hold.origin.y;
            // Only a channel's primary row drags, any channel's.
            hold.dragging |= dy.abs() > m.u(gesture::CALM_U) && lane == 0;
            if hold.dragging {
                let primaries = || -> Vec<usize> {
                    (tracks.iter().filter(|t| t.lane == 0))
                        .map(|t| t.channel)
                        .collect()
                };
                // Every channel shows a row, so its index is below the row count.
                let mut counts = vec![0; tracks.len()];
                for t in tracks {
                    counts[t.channel] += 1;
                }
                let (order, at) = hold.live.get_or_insert_with(|| (primaries(), 0.0));
                let was = (order.clone(), *at);
                // floor(start row) plus the finger's travel in rows (and any scroll since), rounded in `drag_step`.
                let raw = hold.start.floor() + (dy + scroll - hold.scroll) / row_h;
                *at = drag_step(order, &counts, channel, raw);
                if (&*order, *at) != (&was.0, was.1) {
                    ui.ctx().request_repaint();
                }
                if p.long_released && *order != primaries() {
                    actions.reorder = Some(order.clone());
                }
            } else if p.long_released {
                self.track_menu = Some((channel, lane, pos));
            }
        }
        if p.long_released {
            self.hold = None;
        }
        // A secondary tap selects the track, then opens its menu at the click.
        if response.clicked_by(PointerButton::Secondary) {
            self.make_current(app, channel, lane, actions);
            let at = response
                .interact_pointer_pos()
                .unwrap_or(response.rect.center());
            self.track_menu = Some((channel, lane, at));
        }
    }

    /// Where each track row shows: its index in `tracks` and its row from the top (fractional), in drawing order. While a
    /// header drags, in the order the drag left, the dragged channel's rows last, at the finger.
    fn placed(&self, tracks: &[TrackRow]) -> Vec<(usize, f32)> {
        let primaries = tracks.iter().filter(|t| t.lane == 0).count();
        let live = (self.hold.as_ref())
            .and_then(|h| Some((tracks.get(h.row)?.channel, h.live.as_ref()?)))
            .filter(|(_, (order, _))| order.len() == primaries);
        let Some((dragged, (order, at))) = live else {
            return (0..tracks.len()).map(|i| (i, i as f32)).collect();
        };
        let (mut placed, mut floating) = (Vec::new(), Vec::new());
        for (row, i) in (order.iter())
            .flat_map(|&c| (0..tracks.len()).filter(move |&i| tracks[i].channel == c))
            .enumerate()
        {
            match tracks[i].channel == dragged {
                true => floating.push((i, at + floating.len() as f32)),
                false => placed.push((i, row as f32)),
            }
        }
        placed.extend(floating);
        placed
    }

    /// Ends a header's hold: a change of the channels under it (an undo, say) leaves its drag nothing to move.
    pub(crate) fn cancel_hold(&mut self) {
        self.hold = None;
    }

    /// Track `lane` of `channel` becomes the playlist's current track ([`Clips::make_current_track`]: the clips'
    /// taps and the headers share one), and its channel takes the keyboard (never the master's).
    fn make_current(
        &mut self,
        app: &mut AppCtx,
        channel: usize,
        lane: usize,
        actions: &mut ScreenActions,
    ) {
        self.clips.make_current_track(app, channel, lane);
        if channel > 0 {
            actions.header = Some(channel);
        }
    }

    /// The header column's width: 1.5 U + c·12 U.
    fn header_w(&self, m: &Metrics) -> f32 {
        m.u(HEADER_W + HEADER_GROW * self.column.0)
    }

    /// The ruler's gestures over `strip` ([`RangeSelect`]) in song ticks on the Snap `grid` (whole ticks with Snap
    /// off): a tap sets the playhead, and the range is the song's play loop, its edges in the keyboard channel's
    /// colour. A region item is one final edit on every lane ([`region::edit_lanes`]) at the transport's playhead.
    /// Returns the ruler, for the playhead knob.
    fn region(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        strip: Rect,
        axis: TimeAxis,
        grid: Option<f64>,
    ) -> RangeRuler {
        let playhead = app.snapshot.map_or(0, |s| s.transport_tick);
        let theme = app.cx.theme;
        let ink = (app.song.channels.get(*app.selected))
            .map_or(theme.colors.text, |c| channel_color(c.hue, theme));
        let at = RangeRuler {
            strip,
            axis,
            grid,
            sig: app.song.time_sig,
            ink: ink.to_color32(),
            range: (app.song.play_loop).map(|(start, end)| TimeRange { start, end }),
            playhead: Some(playhead as f64),
        };
        let out = self
            .region
            .show(ui, Id::new("playlist_region"), &at, false, &app.cx);
        if let Some(range) = out.range {
            app.song.play_loop = range.map(|r| (r.start, r.end));
        }
        if let Some(tick) = out.playhead {
            app.set_playhead(tick.into());
        }
        let Some((op, range)) = out.pick else {
            return at;
        };
        let before: Vec<_> = app.song.channels.iter().map(|c| c.lanes.clone()).collect();
        let (p, bar) = (
            u32::try_from(playhead).unwrap_or(u32::MAX),
            app.song.time_sig.bar_beats(),
        );
        region::edit_lanes(&mut app.song.channels, op, range, p, bar);
        if !op.keeps_loop() {
            app.song.play_loop = None;
        }
        if app.song.channels.iter().map(|c| &c.lanes).ne(&before) {
            app.song_edited(true);
        }
        at
    }

    /// What every view shares, drawn over it: the keyboard panel (the transport bar and the keyboard) unless it is
    /// hidden. The corner buttons go over the rack ([`Self::corner_buttons`]).
    pub fn chrome(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiCtx,
        input: ScreenInput,
        regions: &Regions,
        actions: &mut ScreenActions,
    ) {
        if !self.panel_hidden {
            // A focus container, for `showTransport`.
            let panel = regions.transport.union(regions.keyboard);
            focus::container(ui, Id::new(focus::TRANSPORT), panel, |ui| {
                self.keyboard_panel(ui, cx, input, regions, actions)
            });
        } else {
            // Hidden: no grab starts, the chord keyboard's notes stop and a bent PITCH springs back.
            self.zone = None;
            actions.chord_notes = self.chords.release_all();
            self.pitch_back(actions);
        }
    }

    /// The buttons on the screen's edges, bottom to top as they stack over the rack: the show-keyboard `↖` (while the
    /// panel is hidden), the rack tab on the right edge (it toggles the rack of `selected`) and the logo corner. The
    /// app draws them over the rack and under every other overlay.
    pub fn corner_buttons(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiCtx,
        selected: usize,
        regions: &Regions,
        actions: &mut ScreenActions,
    ) {
        let UiCtx { theme, m } = *cx;
        let c = &theme.colors;
        // Icons in `menuFill` on `text` discs, as the show-keyboard `↖` is: the logo quarter-disc (top-right) and the
        // rack tab are drawn the same way.
        let (text, ink) = (c.text.to_color32(), c.menu_fill.to_color32());
        self.show_keyboard_button(ui, cx, regions.screen);

        // Centred on the track rows below the ruler.
        let edge_y = (regions.editor.top() + m.u(RULER_H) + regions.editor.bottom()) / 2.0;
        let screen = regions.screen;
        let tab = tab_button(
            ui,
            Id::new("edge_tab"),
            pos2(screen.right(), edge_y),
            2.0 * m.u(EDGE_TAB_R),
            text,
        );
        let tab_icon = pos2(screen.right() - m.u(EDGE_TAB_R) / 2.0, edge_y);
        draw_icon(ui.painter(), at(tab_icon), IconId::Back, ink, TAB_ICON, m);
        if tab.clicked() {
            actions.nav = Some(Nav::Toggle(Overlay::Rack(selected)));
        }

        let logo = tab_button(
            ui,
            Id::new("app_logo"),
            screen.right_top(),
            2.0 * m.u(LOGO_R),
            text,
        );
        let logo_icon = screen.right_top() + vec2(-m.u(LOGO_ICON_INSET), m.u(LOGO_ICON_INSET));
        draw_icon(ui.painter(), at(logo_icon), IconId::AppLogo, ink, 1.0, m);
        if logo.clicked() {
            actions.nav = Some(Nav::Toggle(Overlay::MainMenu(MenuTab::Songs)));
        }
    }

    /// The keyboard panel's height this frame: the first frame takes `target` ([`panel_target`]) at once; then it moves
    /// a [`panel_tick`] per frame and asks for the next frame until it is there.
    pub fn panel_h(&mut self, ctx: &egui::Context, target: f32, m: &Metrics) -> f32 {
        // A hand resize holds the height the finger gave it.
        if let Some(h) = self.panel.filter(|_| self.grab.resizing()) {
            return h;
        }
        let h = self.panel.map_or(target, |h| panel_tick(h, target, m));
        if h != target {
            ctx.request_repaint();
        }
        *self.panel.insert(h)
    }

    /// The channel of id `id` is current: the channel that was current keeps the piano's view, and `id`'s last one
    /// comes back. A channel never current keeps the view as it stands. Session memory only: no file field holds it.
    pub(crate) fn keys_for(&mut self, id: Option<i32>) {
        if id == self.keys_owner {
            return;
        }
        if let Some(old) = self.keys_owner {
            self.keys_views.insert(old, self.keys_view);
        }
        if let Some(&view) = id.and_then(|id| self.keys_views.get(&id)) {
            self.keys_view = view;
        }
        self.keys_owner = id;
    }

    /// The views of the channels whose ids `gone` takes are forgotten, as is the owner if it is one: the view shown stays.
    /// A new song's channels may share the old song's ids; a new channel may take the id of one an undo took away
    /// (undo restores the id counter).
    pub(crate) fn forget_keys_views(&mut self, gone: impl Fn(i32) -> bool) {
        self.keys_views.retain(|&id, _| !gone(id));
        self.keys_owner = self.keys_owner.filter(|&id| !gone(id));
    }

    /// The hand resize of the keyboard panel, before the frame's layout: a grab on the last drawn bar or strip sets the
    /// height at once, so the frame lays the panel out where the finger is; let go, it settles by
    /// [`panel_resize::released`], returned for the settings. `covered`: an overlay other than the rack and the strip
    /// is open (the rack never lies over the bar). Its scroll and zoom move the keys in the chrome.
    pub fn grab(
        &mut self,
        ui: &egui::Ui,
        m: &Metrics,
        covered: bool,
        screen_h: f32,
    ) -> Option<Sizes> {
        let zone = self.zone.clone().filter(|_| !self.panel_hidden)?;
        let grabbed = self.grab.track(ui, &zone, covered, m);
        self.grabbed = grabbed;
        let h = self.panel?;
        let min = panel_resize::panel_min(m);
        if grabbed.grow != 0.0 {
            self.panel = Some((h + grabbed.grow).clamp(min, screen_h.max(min)));
            ui.ctx().request_repaint();
        }
        let last_dy = grabbed.released?;
        Some(Sizes::settled(
            panel_resize::released(h, last_dy, screen_h, m),
            m,
        ))
    }

    /// The grab's zone for the next frame, and the tap toggle: a tap on an empty part of the bar or the strip toggles
    /// by [`panel_resize::toggled`], reported in [`ScreenActions::panel_sizes`].
    fn zone_and_tap(
        &mut self,
        ui: &mut egui::Ui,
        m: &Metrics,
        input: ScreenInput,
        regions: &Regions,
        wheels: bool,
        actions: &mut ScreenActions,
    ) {
        // The scroll zone leaves out the wheels' column while they show.
        let wheels_w = if wheels { m.u(4.0) } else { 0.0 };
        // The strip's knobs take their own drags (`Velocity`, and `Timing` in Chords).
        let knobs = match input.chords {
            Some(c) if !input.pads && input.sample_row.is_none() => {
                let mode = if c.strum {
                    KeyboardMode::Strum
                } else {
                    KeyboardMode::Chords
                };
                knob_rects(regions.strip, true, mode, m)
            }
            _ => Vec::new(),
        };
        let zone = Zone {
            bar: regions.transport,
            strip: regions.strip,
            knobs,
            scroll_left: regions.transport.left() + wheels_w,
        };
        if panel_resize::tap(ui, &zone, m) && actions.panel_sizes.is_none() {
            let s = input.settings;
            let screen_h = regions.screen.height();
            let (preferred, recent) = (m.u(s.preferred_kbd_size), m.u(s.recent_kbd_size));
            actions.panel_sizes = Some(Sizes {
                preferred: panel_resize::toggled(preferred, recent, screen_h, m),
                recent: None,
            });
        }
        self.zone = Some(zone);
    }

    /// The keyboard panel: the transport bar with the editors' buttons, and the keyboard (the drum view has its pads
    /// there instead, and an audio clip its sample row).
    fn keyboard_panel(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiCtx,
        input: ScreenInput,
        regions: &Regions,
        actions: &mut ScreenActions,
    ) {
        let panel = Rect::from_min_max(regions.transport.min, regions.keyboard.max);
        let shown = wheels_shown(input, panel, cx.m);
        keyboard::panel_backdrop(ui.painter(), panel, shown, cx);
        self.zone_and_tap(ui, cx.m, input, regions, shown, actions);
        let grabbed = std::mem::take(&mut self.grabbed);
        self.transport(ui, regions, cx, input, actions);
        let underline = (input.tracks.iter().find(|t| t.channel == input.selected))
            .map_or(cx.theme.colors.text, |t| channel_color(t.hue, cx.theme));
        let ink = underline.to_color32();
        // The chord keyboard, while it shows instead of the piano (a keys area exactly 0 tall hides it).
        let chords = input.chords.filter(|_| {
            !input.pads && input.sample_row.is_none() && regions.keyboard.height() != 0.0
        });
        match chords {
            Some(chords) => {
                let rect =
                    chord_keyboard::chord_rect(regions.keyboard, input.settings.show_wheels, cx.m);
                if chords.editor {
                    // The strips shrink to the bottom; the editor edits the target strip's chord from the keys' top
                    // down, under the strips, which are drawn over it and take the touches there first. It shows the
                    // strip the last frame left targeted.
                    let (editor, strips) = chord_editor::editor_rects(rect, cx.m);
                    let target = self.chords.target.min(chords.set.chords.len() - 1);
                    let chord = &chords.set.chords[target];
                    let edited = chord_editor::chord_editor(ui, cx, editor, chord);
                    actions.chord_notes = self.chords.show(ui, cx, strips, underline, chords);
                    actions.chord_edit = edited.chord.map(|c| (target, c));
                    let pitches = edited.held.map(|c| c.inversion(0, chords.shift));
                    let notes = self.chords.editor_voice(pitches, chords.velocity);
                    actions.chord_notes.extend(notes);
                } else {
                    // A button held as the editor closed (another finger's `Close Editor`) lets its chord go.
                    actions.chord_notes = self.chords.editor_voice(None, chords.velocity);
                    let notes = self.chords.show(ui, cx, rect, underline, chords);
                    actions.chord_notes.extend(notes);
                }
            }
            None => actions.chord_notes = self.chords.release_all(),
        }
        match input.sample_row {
            _ if input.pads => {}
            _ if chords.is_some() => {}
            Some(row) => {
                actions.sample_row = sample_row::show(ui, cx, regions.keyboard, ink, row);
            }
            None => {
                let rect = piano_rect(regions.keyboard, shown, cx.m);
                if !self.keys_initialized && rect.is_positive() {
                    let middle_c = keyboard::slots(0..=59, None) + 0.5;
                    self.keys_view.left =
                        middle_c - rect.width() / (2.0 * cx.m.u(self.keys_view.white_w));
                    self.keys_initialized = true;
                }
                // The second row shows over a panel taller than 5 U.
                let piano = keyboard::Piano {
                    rect,
                    two_rows: keyboard::second_row(
                        input.piano,
                        input.settings.show_two_rows,
                        panel,
                        cx.m,
                    ),
                    view: self.keys_view,
                };
                // The scroll zone's moves, kept in range for the layout shown.
                self.keys_view = piano.moved(grabbed.scroll, grabbed.zoom, cx.m);
                let piano = keyboard::Piano {
                    view: self.keys_view,
                    ..piano
                };
                actions.keys =
                    keyboard::keyboard(ui, piano, cx, ink, input.settings, input.keyboard);
            }
        }
        if shown {
            let rects = wheel_rects(panel, cx.m);
            let enabled = input.keyboard.enabled;
            actions.wheels = wheels::wheels(ui, cx, rects, ink, input.wheels, enabled);
            if let Some(&(_, v)) = (actions.wheels.moves.iter()).rfind(|m| m.0 == wheels::PITCH) {
                self.pitch_bent = v != 0.5;
            }
        } else {
            self.pitch_back(actions);
        }
    }

    /// A frame without the wheels, which so never see a PITCH drag end (the panel hidden, toggled down, the setting
    /// off): a bent PITCH springs back to 0.5, as its release would.
    fn pitch_back(&mut self, actions: &mut ScreenActions) {
        if std::mem::take(&mut self.pitch_bent) {
            actions.wheels.moves.push((wheels::PITCH, 0.5));
        }
    }

    /// While the keyboard panel is hidden, the show-keyboard `↖`: a [`LOGO_R`] quarter-disc centred on the screen's
    /// bottom-right corner, filled with `text`, with icon 11 in `menuFill`, pressed as the logo is. A tap shows the
    /// panel again (the screen re-lays out from the next frame).
    fn show_keyboard_button(&mut self, ui: &mut egui::Ui, cx: &UiCtx, screen: Rect) {
        if !self.panel_hidden {
            return;
        }
        let (c, m) = (&cx.theme.colors, cx.m);
        let corner = screen.right_bottom();
        let id = Id::new("show_keyboard");
        let button = tab_button(ui, id, corner, 2.0 * m.u(LOGO_R), c.text.to_color32());
        let icon = corner - Vec2::splat(m.u(LOGO_ICON_INSET));
        let ink = c.menu_fill.to_color32();
        draw_icon(ui.painter(), at(icon), IconId::Detach, ink, 1.0, m);
        if button.clicked() {
            self.panel_hidden = false;
            ui.ctx().request_repaint();
        }
    }

    /// The scroll and zoom controller over the playlist: touch scroll y over the track `area`, touch scroll x over the
    /// grid columns from the `ruler` top down; the wheel zooms the rows over the headers and the time over the ruler,
    /// and scrolls both over the grid, where a trackpad pinch zooms the time; two fingers zoom the axes of the first
    /// one's zones. With `follow` (the playhead, in beats) the view's left edge keeps the playhead a quarter in while
    /// the song is longer than the view, and one-finger horizontal scrolling is off. Any horizontal scroll or zoom
    /// hides `+Clip` and the ruler's region `+` and cancels a ruler press waiting for its long press. No new touch,
    /// wheel or pinch is the playlist's while one of its popups is open, nor the wheel and pinch without `wheel` (an
    /// overlay covers the pointer). A track header's press holds the rows only once its hold has taken the touch (its
    /// calm zone is the scroller's y one). The headers' column is [`Self::header_w`] wide. Returns the vertical offset
    /// snapped to whole device pixels (for painting).
    fn scroll(
        &mut self,
        ui: &mut egui::Ui,
        ruler: Rect,
        area: Rect,
        content: Content,
        (follow, wheel): (Option<f64>, bool),
        m: &Metrics,
    ) -> f32 {
        let headers = area.with_max_x(area.left() + self.header_w(m));
        let grid = area.with_min_x(headers.right() + m.t());
        // The playlist's grid holds the focus for its editing actions.
        focus::register(ui, Id::new("playlist grid"), grid, focus::Kind::Container);
        let ruler = ruler.with_min_x(grid.left());
        let zones = Zones {
            touch: [ruler.union(grid), area],
            // Not the rows under a held header: its drag measures rows at their height then.
            wheel_zoom: [
                ruler,
                match self.hold {
                    Some(_) => Rect::NOTHING,
                    None => headers,
                },
            ],
            wheel_scroll: grid,
            pinch: grid,
        };
        let mut axes = self.axes(area, content, m);
        let was = axes;
        let follow = follow.filter(|_| content.end > axes[X].span());
        let popup = self.add_channel_open
            || self.track_menu.is_some()
            || self.insert.is_some()
            || self.snap_menu;
        let opts = scroller::Options {
            covered: popup || !wheel,
            follow: follow.is_some(),
        };
        let id = Id::new("track_scroll");
        let report = (self.scroller).update(ui, id, m.u(1.0), &zones, &mut axes, opts);
        if let Some(playhead) = follow {
            scroller::follow(&mut axes[X], playhead);
        }
        self.take(axes, was, m);
        // The rows' range counts the `+` room in points: a row zoom changes it, and the zoom clamped the position to the
        // range before it, so it is kept to the new one now (else it springs back from past the end).
        if axes[Y].zoom != was[Y].zoom {
            let [_, y] = self.axes(area, content, m);
            let pos = y.pos.clamp(y.range[0], y.max());
            if pos != y.pos {
                self.scroll_y = (pos / y.zoom) as f32;
            }
        }
        if report.scrolled[X] || report.zoomed[X] {
            self.clips.view_moved();
            self.region.view_moved();
        }
        m.round_px(self.scroll_y)
    }

    /// The view as the scroller's axes: x in beats over the grid, over 0 … 2·(song end + 1/128); y in rows over the
    /// track `area`, over the rows and the room for the `+`.
    fn axes(&self, area: Rect, c: Content, m: &Metrics) -> [Axis; 2] {
        let grid = area.with_min_x(area.left() + self.header_w(m) + m.t());
        let (beat_w, row_h) = (self.axis(0.0, m).beat_w, self.row_h(m));
        let range = 2.0 * (c.end + 1.0 / 128.0);
        let x = Axis {
            pos: self.scroll_x,
            zoom: 1.0 / f64::from(beat_w),
            range: [0.0, range],
            zoom_range: time_zoom(c.d, m),
            view: grid.x_range(),
        };
        let zoom = 1.0 / f64::from(row_h);
        let y = Axis {
            pos: f64::from(self.scroll_y / row_h),
            zoom,
            range: [0.0, rows_range(c.rows, zoom, m)],
            zoom_range: [ROW_RANGE[1], ROW_RANGE[0]].map(|u| 1.0 / f64::from(m.u(u))),
            view: area.y_range(),
        };
        [x, y]
    }

    /// Takes back what changed in `axes` since `was` ([`Self::axes`]).
    fn take(&mut self, [x, y]: [Axis; 2], [wx, wy]: [Axis; 2], m: &Metrics) {
        if x.zoom != wx.zoom {
            self.zoom = ((1.0 / x.zoom) as f32 / m.u(BEAT_W)).log2();
        }
        if x.pos != wx.pos {
            self.scroll_x = x.pos;
        }
        if y.zoom != wy.zoom {
            self.row_u = Some((1.0 / y.zoom) as f32 / m.u(1.0));
        }
        if y != wy {
            self.scroll_y = (y.pos / y.zoom) as f32;
        }
    }

    /// What a view change is told by.
    fn view(&self) -> (f64, f32, f32, Option<f32>) {
        (self.scroll_x, self.scroll_y, self.zoom, self.row_u)
    }

    /// The view the song stores (HEAD@296), applied on open in this order: zoom y, scroll y, zoom x, scroll x. The
    /// values are device pixels with no U scaling (zoom x in beats a pixel, zoom y in rows a pixel, scroll x in beats,
    /// scroll y in rows). Each zoom goes through its clamp (zoom x also through the range/view cap, so a short song
    /// zooms in further) and is written back to the song at once, at f32 precision; each scroll is clamped by the view
    /// only, the song keeping it as stored until the view next moves. A zoom that is not a positive number reads as the
    /// default (0.01, 0.02), a scroll that is not finite as 0.
    fn restore_view(&mut self, stored: &mut [f64; 4], area: Rect, c: Content, m: &Metrics) {
        // The old song's fling or spring goes with it.
        self.scroller = Scroller::default();
        let [x, y, zoom_x, zoom_y] = *stored;
        let ppp = f64::from(m.ppp());
        let zoom = |v: f64, default: f64| if v.is_finite() && v > 0.0 { v } else { default } * ppp;
        let at = |v: f64| if v.is_finite() { v } else { 0.0 };
        let [mut ax, mut ay] = self.axes(area, c, m);
        ay.zoom = ay.clamp_zoom(zoom(zoom_y, 0.02));
        ay.range[1] = rows_range(c.rows, ay.zoom, m);
        ay.pos = at(y);
        ay = ay.clamped();
        // The range/view cap: no more than the whole range shows.
        ax.zoom_range[1] = ax.zoom_range[1].min(ax.range[1] / f64::from(ax.view.span()));
        ax.zoom = ax.clamp_zoom(zoom(zoom_x, 0.01));
        ax.pos = at(x);
        ax = ax.clamped();
        stored[2] = f64::from((ax.zoom / ppp) as f32);
        stored[3] = f64::from((ay.zoom / ppp) as f32);
        self.zoom = ((1.0 / ax.zoom) as f32 / m.u(BEAT_W)).log2();
        self.scroll_x = ax.pos;
        self.row_u = Some((1.0 / ay.zoom) as f32 / m.u(1.0));
        self.scroll_y = (ay.pos / ay.zoom) as f32;
    }

    /// A track row's height, in points.
    pub(crate) fn row_h(&self, m: &Metrics) -> f32 {
        m.u(self.row_u.unwrap_or(ROW_H))
    }

    /// The timeline of a grid whose left edge is at `grid_left`, scrolled and zoomed.
    fn axis(&self, grid_left: f32, m: &Metrics) -> TimeAxis {
        let beat_w = m.u(BEAT_W) * self.zoom.exp2();
        TimeAxis {
            origin: grid_left - self.scroll_x as f32 * beat_w,
            beat_w,
        }
    }

    /// Shows the add-channel popup if it is open, centred on `anchor`. Returns the picked channel kind.
    fn add_channel_popup(
        &mut self,
        ctx: &egui::Context,
        anchor: Pos2,
        screen: Rect,
        cx: &UiCtx,
    ) -> Option<ChannelKind> {
        if !self.add_channel_open {
            return None;
        }
        let id = Id::new("add_channel_popup");
        let result = popup::show(ctx, id, anchor, screen, &add_channel_menu(), cx);
        if result.close {
            self.add_channel_open = false;
            // Nothing else triggers a frame to redraw without the popup, or with the new row.
            ctx.request_repaint();
        }
        result.selected
    }

    /// Shows the track menu if it is open. Returns its pick with the track; `Insert Channel` opens the add-channel
    /// popup at the same place instead ([`Self::insert_popup`]).
    fn track_menu(
        &mut self,
        ctx: &egui::Context,
        song: &Song,
        screen: Rect,
        cx: &UiCtx,
    ) -> Option<(usize, usize, TrackOp)> {
        let (channel, lane, at) = self.track_menu?;
        let items = track_menu(song, channel, lane);
        let result = popup::show(ctx, Id::new("track_menu"), at, screen, &items, cx);
        if result.close || items.is_empty() {
            self.track_menu = None;
            ctx.request_repaint();
        }
        match result.selected? {
            TrackOp::InsertChannel => {
                self.insert = Some((channel, at));
                None
            }
            op => Some((channel, lane, op)),
        }
    }

    /// `Insert Channel`'s add-channel popup, while it is open. Returns the picked kind with the index it goes in at:
    /// the long-pressed channel's, so the new channel lands above it.
    fn insert_popup(
        &mut self,
        ctx: &egui::Context,
        screen: Rect,
        cx: &UiCtx,
    ) -> Option<(usize, ChannelKind)> {
        let (at, center) = self.insert?;
        let id = Id::new("insert_channel_popup");
        let result = popup::show(ctx, id, center, screen, &add_channel_menu(), cx);
        if result.close {
            self.insert = None;
            ctx.request_repaint();
        }
        result.selected.map(|kind| (at, kind))
    }

    /// The transport bar, its buttons in the slots of [`transport_slots`]. Sets REV's click, the navigation its
    /// buttons ask for (the transport's before any other), the editors' `Undo` / `Redo` step and `Preview` click, and
    /// a tap on the CPU or RAM readout ([`ScreenActions::performance`]).
    fn transport(
        &mut self,
        ui: &mut egui::Ui,
        regions: &Regions,
        cx: &UiCtx,
        input: ScreenInput,
        actions: &mut ScreenActions,
    ) {
        let rect = regions.transport;
        let cpu = cpu_share(input.audio);
        let (c, m) = (&cx.theme.colors, cx.m);
        // Its `controlFill` is the keyboard panel's backdrop ([`keyboard::panel_backdrop`]).
        let (text, bg) = (c.text.to_color32(), c.background.to_color32());
        let d = m.u(ROUND_BUTTON_D);
        // The editors' buttons: `Undo` while there is something to undo, `Redo` while there is something to redo, and
        // `Preview` while the editor edits a clip.
        let editor = !matches!(input.view, View::Playlist | View::Mixer);
        let (can_undo, can_redo) = input.history;
        let shown = [editor && can_undo, input.can_preview, editor && can_redo];
        let top = rect.top() - regions.screen.top();
        let meters = input.settings.hardware_meters;
        let slots = transport_slots(rect, top, meters, shown, m);

        // Left: mixer and keyboard settings.
        let mut nav = None;
        for (slot, (name, icon, target)) in slots.left.into_iter().zip([
            ("mixer", IconId::Mixer, Nav::Show(View::Mixer)),
            (
                "keyboard_settings",
                IconId::KeyboardSettings,
                Nav::Toggle(Overlay::KeyboardSettings),
            ),
        ]) {
            let id = Id::new(("transport", name));
            let button = outline_circle_button(ui, id, slot.center(), d, text, m);
            draw_icon(ui.painter(), button.rect(), icon, text, TRANSPORT_ICON, m);
            if button.clicked() {
                nav = Some(target);
            }
        }

        // Centre group: REC, REV, play, tempo, CTRL.
        // REC: a plain arm toggle, hint `Arm recording`. Armed, its icon is in the record colour; its caption stays in
        // `text`.
        let id = Id::new(("transport", "REC"));
        let (rec, content) = captioned_button(ui, id, slots.rec, "REC", text, m);
        if rec.clicked() {
            self.armed = !self.armed;
        }
        let armed = self.armed;
        rec.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, armed, "Arm recording")
        });
        let ink = if armed {
            RECORD_COLOR.to_color32()
        } else {
            text
        };
        draw_icon(
            ui.painter(),
            content,
            IconId::Record,
            ink,
            TRANSPORT_ICON,
            m,
        );
        for (slot, icon, caption) in [
            (slots.rev, IconId::Rewind, "REV"),
            (slots.ctrl, IconId::More, "CTRL"),
        ] {
            let id = Id::new(("transport", caption));
            let (response, content) = captioned_button(ui, id, slot, caption, text, m);
            draw_icon(ui.painter(), content, icon, text, TRANSPORT_ICON, m);
            actions.rewind |= caption == "REV" && response.clicked();
            if caption == "CTRL" && response.clicked() {
                nav = Some(Nav::Toggle(Overlay::CtrlMenu));
            }
        }
        let play_id = Id::new(("transport", "play"));
        let play = outline_circle_button(ui, play_id, slots.play.center(), d, text, m);
        if play.clicked() {
            self.playing = !self.playing;
        }
        if self.playing {
            pause_glyph(ui.painter(), play.rect().center(), text, m);
        } else {
            draw_icon(
                ui.painter(),
                play.rect(),
                IconId::Play,
                text,
                TRANSPORT_ICON,
                m,
            );
        }
        let (bpm, tempo) =
            captioned_button(ui, Id::new(("transport", "BPM")), slots.bpm, "BPM", text, m);
        if bpm.clicked() {
            nav = Some(Nav::Toggle(Overlay::TempoEditor));
        }
        let font = m.font_id(FontSize::Pt12);
        let bpm = transport_bpm_text(input.bpm);
        ui.painter()
            .text(tempo.center(), Align2::CENTER_CENTER, bpm, font, text);

        // Right: the close X, then right to left RAM, CPU, Preview, Redo, Undo.
        let close_id = Id::new(("transport", "close"));
        let close = filled_circle_button(ui, close_id, slots.close.center(), d, text);
        draw_icon(
            ui.painter(),
            close.rect(),
            IconId::Close,
            bg,
            TRANSPORT_ICON,
            m,
        );
        // It only hides the keyboard panel, in every view; the `↖` brings it back. The view and the overlays stay (the
        // clip editors have their own close X).
        if close.clicked() {
            self.panel_hidden = true;
        }
        // The CPU and RAM meters (setting `Show CPU and RAM meters`) in their slots: the CPU bar on its blink ramp, the
        // RAM readout, and the labels' tooltips; a tap on either opens PROJECT's Performance block.
        if let Some(slot) = slots.cpu {
            let painter = ui.painter();
            let line = |v: f32| slot.top() + m.u(v);
            let x = slot.center().x;
            let font = m.font_id(FontSize::Pt10);
            let label = pos2(x, line(READOUT_LINES[0]));
            painter.text(label, Align2::CENTER_CENTER, "CPU", font, text);
            let size = vec2(m.u(CPU_BAR[0]), m.u(CPU_BAR[1]));
            let cpu_bar = Rect::from_center_size(pos2(x, line(READOUT_LINES[1])), size);
            let now = ui.input(|i| i.time);
            let background = cx.theme.colors.background.to_color32();
            painter.rect_filled(
                cpu_bar,
                0.0,
                cpu_blink(self.meter.smoothed, now, background),
            );
            if self.meter.smoothed >= BLINK_FROM {
                ui.ctx().request_repaint_after(Duration::from_millis(250));
            }
            let filled = cpu_bar.with_max_x(cpu_bar.left() + cpu * cpu_bar.width());
            painter.rect_filled(filled, 0.0, CPU_GREEN);
        }
        if let Some(slot) = slots.ram {
            self.ram.show(ui, cx, slot);
        }
        if slots.cpu.is_some() || slots.ram.is_some() {
            // The readout reads the memory on frames only.
            ui.ctx().request_repaint_after(Duration::from_secs(1));
            let none = Rect::NOTHING;
            let labels = [
                (Meter::Cpu, slots.cpu.unwrap_or(none)),
                (Meter::Ram, slots.ram.unwrap_or(none)),
            ];
            let memory = (self.ram.text(), self.ram.system());
            let load = (self.meter.smoothed, self.meter.peak);
            if self.meters.show(ui, cx, rect, labels, load, memory) {
                self.performance_tap = true;
                nav = Some(Nav::Open(Overlay::MainMenu(MenuTab::Project)));
            }
        }
        if let Some(slot) = slots.preview.filter(|_| shown[1]) {
            actions.preview = preview_button(ui, cx, slot, input.preview.is_some());
        }
        // Undo/Redo as light bubbles, not `menuFill` text buttons: the Preview circle's size in their slots, lit while
        // held or hovered, labelled as the floating row's bubbles are (10 pt overflows the disc).
        let font = label_font(ui.ctx(), m, m.u(PREVIEW_D), &["Undo", "Redo"]);
        for (slot, label, step) in [
            (Some(slots.undo).filter(|_| shown[0]), "Undo", Step::Undo),
            (slots.redo.filter(|_| shown[2]), "Redo", Step::Redo),
        ] {
            let Some(slot) = slot else { continue };
            let (id, d) = (Id::new(("transport", label)), m.u(PREVIEW_D));
            if bubble_button(ui, id, slot.center(), d, 1.0, label, font.clone(), c).clicked() {
                actions.step = Some(step);
            }
        }
        actions.nav = nav.or(actions.nav.take());
    }
}

/// The pitch and mod wheels show: the `showWheels` setting is on, the keyboard `panel` is over 5 U tall, and the keys
/// are a piano or the chord keyboard, not the pads or the sample row.
fn wheels_shown(input: ScreenInput, panel: Rect, m: &Metrics) -> bool {
    input.settings.show_wheels
        && panel.height() > m.u(5.0)
        && !input.pads
        && input.sample_row.is_none()
}

/// The transport bar's slots: each b × b, b = 1.75 U, at the bar's top. The right group's are `None` while they have no
/// slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportSlots {
    /// Mixer and keyboard settings.
    pub left: [Rect; 2],
    /// The drum view's `Lock`, right of the keyboard settings.
    pub lock: Rect,
    pub rec: Rect,
    pub rev: Rect,
    pub play: Rect,
    pub bpm: Rect,
    pub ctrl: Rect,
    pub close: Rect,
    pub ram: Option<Rect>,
    pub cpu: Option<Rect>,
    pub preview: Option<Rect>,
    pub redo: Option<Rect>,
    pub undo: Rect,
}

/// The slots of the transport bar `bar`, whose top is `top` points below the screen's. `meters`: the CPU and RAM meters
/// setting; `shown`: which of `Undo`, `Preview` and `Redo` show.
/// - Centre: Play at the bar's centre − b/2, REV and REC one and two b left of it, BPM at the centre + b/2 and CTRL
///   one b right of BPM.
/// - Left: Mixer at the bar's left (the app's window has no top margin, so it is never 2.75 U in), then the keyboard
///   settings and `Lock`.
/// - The close X at the right edge, or 3.75 U further left while the bar's top is less than 1.5 U from the screen's.
/// - Right to left from the close X: [`right_group`].
pub fn transport_slots(
    bar: Rect,
    top: f32,
    meters: bool,
    shown: [bool; 3],
    m: &Metrics,
) -> TransportSlots {
    let b = m.u(SLOT_W);
    let slot = |left: f32| Rect::from_min_size(pos2(left, bar.top()), Vec2::splat(b));
    let centre = |i: f32| transport_slot(bar, i, m);
    let right = bar.right() - if top < m.u(1.5) { m.u(3.75) } else { 0.0 };
    let close = slot(right - b);
    let ctrl = centre(CTRL_SLOT);
    let n = ((close.left() - ctrl.right()) / b).floor().max(0.0) as usize;
    let at = |k: Option<usize>| k.map(|k| slot(close.left() - (k + 1) as f32 * b));
    let [ram, cpu, preview, redo, undo] = right_group(n, meters, shown).map(at);
    TransportSlots {
        left: [slot(bar.left()), slot(bar.left() + b)],
        lock: slot(bar.left() + 2.0 * b),
        rec: centre(0.0),
        rev: centre(1.0),
        play: centre(2.0),
        bpm: centre(3.0),
        ctrl,
        close,
        ram,
        cpu,
        preview,
        redo,
        undo: undo.expect("Undo always takes a slot"),
    }
}

/// The right group's slots, counted right to left from the close X (0 is next to it), for RAM, CPU, Preview, Redo and
/// Undo: `n` slots fit between CTRL and the close X; s = 6 with the `meters` on, else 3; m = the buttons `shown` among
/// `Undo`, `Preview`, `Redo`, plus 3 with the meters on.
/// - RAM, then CPU: only with the meters on, and not when s > n and m > n; each one dropped lowers m by 1.
/// - Preview keeps its slot if n ≥ s, or if it shows and m ≤ n; otherwise m −= 1.
/// - Redo keeps its slot if n ≥ s, or if it shows and m ≤ n.
/// - Undo always takes the next slot.
pub fn right_group(n: usize, meters: bool, shown: [bool; 3]) -> [Option<usize>; 5] {
    let [_, preview, redo] = shown;
    let s = if meters { 6 } else { 3 };
    let mut m = shown.iter().filter(|&&v| v).count() + if meters { 3 } else { 0 };
    let mut keeps = [false; 5];
    for meter in &mut keeps[..2] {
        *meter = meters && !(s > n && m > n);
        if meters && !*meter {
            m -= 1;
        }
    }
    keeps[2] = n >= s || (preview && m <= n);
    if !keeps[2] {
        m = m.saturating_sub(1);
    }
    keeps[3] = n >= s || (redo && m <= n);
    keeps[4] = true;
    let mut next = 0;
    keeps.map(|keep| {
        keep.then(|| {
            next += 1;
            next - 1
        })
    })
}

/// The `Preview` toggle centred in `slot`: a [`PREVIEW_D`] outline circle around its 6 pt label, lit (a `text` disc
/// behind a `background` label) while `on`, as the Modules browser's `Search` toggle is. Returns whether it was
/// clicked.
fn preview_button(ui: &mut egui::Ui, cx: &UiCtx, slot: Rect, on: bool) -> bool {
    let (c, m) = (&cx.theme.colors, cx.m);
    let (text, bg) = (c.text.to_color32(), c.background.to_color32());
    let (center, d) = (slot.center(), m.u(PREVIEW_D));
    let id = Id::new(("transport", "Preview"));
    let clicked = outline_circle_button(ui, id, center, d, text, m).clicked();
    let ink = if on {
        ui.painter().circle_filled(center, d / 2.0, text);
        bg
    } else {
        text
    };
    let font = m.font_id(FontSize::Pt6);
    ui.painter()
        .text(center, Align2::CENTER_CENTER, "Preview", font, ink);
    clicked
}

/// Slot `i` of the transport's centre group in the transport bar `rect`: REC is 0, play 2, CTRL 4.
fn transport_slot(rect: Rect, i: f32, m: &Metrics) -> Rect {
    let slot_w = m.u(SLOT_W);
    let left = rect.center().x + (i - 2.5) * slot_w;
    Rect::from_x_y_ranges(left..=left + slot_w, rect.y_range())
}

/// The CPU bar's filled share: the audio callback's load clamped to `0..=1`, and empty without audio or once
/// the stream has failed.
fn cpu_share(audio: Option<&OutputSnapshot>) -> f32 {
    match audio {
        // `>` is false for NaN too.
        Some(s) if !s.stream_failed && s.cpu_load > 0.0 => s.cpu_load.min(1.0),
        _ => 0.0,
    }
}

/// One drawn frame of the header column's expansion `c` toward `t`.
fn column_step(c: f32, t: f32) -> f32 {
    let c = c + COLUMN_RATE * (t - c);
    if (c - t).abs() < COLUMN_SNAP { t } else { c }
}

/// The Snap button's rect in the ruler row's top-left: (x, top, 1.5 U, ruler h − 1 px).
fn snap_rect(ruler: Rect, m: &Metrics) -> Rect {
    Rect::from_min_size(
        ruler.min,
        vec2(m.u(HEADER_W), ruler.height() - 1.0 / m.ppp()),
    )
}

/// What the playlist scrolls over: the track rows, the song's end in beats ([`song_end_beats`]) and d = TDIV byte 0.
#[derive(Clone, Copy, Debug)]
struct Content {
    rows: usize,
    end: f64,
    d: u8,
}

/// The playlist's time zoom limits in beats a point: (0.01/U)·4/d … (12/U)·4/d beats a pixel, d = TDIV byte 0 (0 read
/// as 1), so a beat is d/48 U … 25·d U wide. (The song's open also caps zoomMax at range/view width, only there:
/// [`MainScreen::restore_view`].)
fn time_zoom(d: u8, m: &Metrics) -> [f64; 2] {
    let du = f64::from(d.max(1)) * f64::from(m.u(1.0));
    [1.0 / (25.0 * du), 48.0 / du]
}

/// The rows' scroll range, in rows (`zoom` rows a point): every row, then the room for the `+`.
fn rows_range(rows: usize, zoom: f64, m: &Metrics) -> f64 {
    rows as f64 + f64::from(m.u(ADD_ROOM)) * zoom
}

#[cfg(test)]
/// Height of everything the track area scrolls: all rows (`row_h` points each), then room for the `+`.
fn content_height(rows: usize, row_h: f32, m: &Metrics) -> f32 {
    rows as f32 * row_h + m.u(ADD_ROOM)
}

/// The rows (of `count`, each `row_h` tall) that touch a view `view_h` tall scrolled by `offset`.
fn visible_rows(offset: f32, view_h: f32, row_h: f32, count: usize) -> Range<usize> {
    let first = (offset / row_h).floor().max(0.0) as usize;
    let end = ((offset + view_h) / row_h).ceil().max(0.0) as usize;
    first.min(count)..end.min(count)
}

/// Top edge of row `index` when the rows (`row_h` points each) are scrolled by `offset`.
fn row_top(area: Rect, offset: f32, index: usize, row_h: f32) -> f32 {
    area.top() - offset + index as f32 * row_h
}

/// Centre of the add-channel `+` below `rows` track rows ending at `rows_bottom`: on the centre of the whole `screen`,
/// not of the playlist, so the open rack never moves it. [`ADD_GAP`] below the last row, scrolling with the rows; in an
/// empty song (the master row alone) midway between that row and the bottom of `track_area`, in the middle of the
/// playlist.
fn add_button_center(
    screen: Rect,
    track_area: Rect,
    rows: usize,
    rows_bottom: f32,
    m: &Metrics,
) -> Pos2 {
    let gap = if rows > 1 {
        m.u(ADD_GAP)
    } else {
        (track_area.bottom() - rows_bottom - m.u(ADD_D)).max(m.u(2.0 * ADD_GAP)) / 2.0
    };
    pos2(screen.center().x, rows_bottom + gap + m.u(ADD_D) / 2.0)
}

/// The bars of the add-channel plus centred on `center`: horizontal, then vertical. A plain plus with no ring,
/// [`PLUS_LEN`] long and `t` thick, on whole pixels; the length takes the stroke's parity so the arms are equal.
fn plus_bars(center: Pos2, m: &Metrics) -> [Rect; 2] {
    let ppp = m.ppp();
    let t = m.t() * ppp;
    let arm = ((m.u(PLUS_LEN) * ppp - t) / 2.0).round();
    // The square where the bars cross, in device pixels.
    let cross = Rect::from_min_size(
        (center * ppp - Vec2::splat(t / 2.0)).round(),
        Vec2::splat(t),
    );
    [cross.expand2(vec2(arm, 0.0)), cross.expand2(vec2(0.0, arm))].map(|r| r / ppp)
}

/// A zero-size rect at `p`, to centre an icon on a point.
fn at(p: Pos2) -> Rect {
    Rect::from_min_size(p, Vec2::ZERO)
}

/// Paints `rect` in `background`, then the lanes of `tracks`, each from its top edge, in order (rows above or below the
/// rect are clipped by the painter, so a scrolled caller passes just the visible rows), with the beat grid of `axis`
/// from `grid_left` on. The header column left of it is [`mini_mixer::show`]'s. Each row is filled from the grid's left
/// edge (or beat 0's x when scrolled before it) to its right edge, a device pixel short of the next row, in
/// `moduleFill` for the `current` track and `controlFill` for the others; the beat lines over them are `background`,
/// 4 px at bars, 2 px at beats and 1 px finer, centred, at least 0.6 U apart (the drum free grid's [`marks`]).
fn paint_track_rows(
    painter: &Painter,
    rect: Rect,
    grid_left: f32,
    tracks: &[(TrackRow, f32)],
    axis: TimeAxis,
    (sig, row_h, current): (TimeSig, f32, Option<(usize, usize)>),
    cx: &UiCtx,
) {
    let UiCtx { theme, m } = *cx;
    let c = &theme.colors;
    let bg = c.background.to_color32();
    painter.rect_filled(rect, 0.0, bg);
    let grid = rect.with_min_x(grid_left);
    let px = 1.0 / m.ppp();
    let (t0, t1) = (axis.tick(grid.left()), axis.tick(grid.right()));
    let lines: Vec<(f32, f32)> = marks(t0, t1, axis.beat_w, m.u(1.0), sig, MARK_GAP)
        .into_iter()
        .map(|(tick, w)| (m.round_px(axis.x(tick)) - (w / 2.0).floor() * px, w * px))
        .collect();
    let grid_painter = painter.with_clip_rect(painter.clip_rect().intersect(grid));
    for &(ref track, top) in tracks {
        // A 1-px `background` gap under each row.
        let lane = Rect::from_x_y_ranges(grid.x_range(), top..=top + row_h - px);
        let fill = match current == Some((track.channel, track.lane)) {
            true => c.module_fill,
            false => c.control_fill,
        };
        let from = lane.left().max(axis.x(0.0));
        painter.rect_filled(lane.with_min_x(from), 0.0, fill.to_color32());
        for &(x, w) in &lines {
            let line = Rect::from_x_y_ranges(x..=x + w, lane.y_range());
            grid_painter.rect_filled(line, 0.0, bg);
        }
    }
}

/// The armed multitrack dot of `header`: its centre and radius.
pub(crate) fn armed_dot(header: Rect, m: &Metrics) -> (Pos2, f32) {
    (
        header.left_top() + Vec2::splat(m.u(ARMED_DOT_AT)),
        m.u(ARMED_DOT_R),
    )
}

/// Two bars standing in for icon 3 while playing.
fn pause_glyph(painter: &Painter, center: Pos2, color: Color32, m: &Metrics) {
    let size = vec2(m.u(PAUSE_BAR[0]), m.u(PAUSE_BAR[1]));
    for side in [-1.0, 1.0] {
        let bar = Rect::from_center_size(center + vec2(side * size.x, 0.0), size);
        painter.rect_filled(bar, 0.0, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_link::AudioLink;
    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::project_files::ProjectFiles;
    use crate::project_files::mem::MemFiles;
    use crate::song::{Event, LaneKind, Song, SynthKind};
    use crate::theme::{Rgba, theme_by_id};
    use crate::ui::clips::ClipRef;
    use crate::ui::offscreen::Offscreen;
    use crate::ui::views::{AppCtx, SongEdit};

    thread_local! {
        /// The transport plays at this song tick with Settings `Continuous scrolling during playback` on, in [`frame`].
        static FOLLOW: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
        /// Files in the user folder, in [`frame`]: (path, bytes).
        static USER_FILES: std::cell::RefCell<Vec<(&'static str, Vec<u8>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    #[test]
    fn each_channel_gets_its_last_keyboard_view_back() {
        let mut screen = MainScreen::default();
        let a = keyboard::KeyView {
            left: 40.0,
            white_w: 3.0,
        };
        let b = keyboard::KeyView {
            left: 10.0,
            white_w: 1.5,
        };
        screen.keys_for(Some(1));
        screen.keys_view = a;
        screen.keys_for(Some(2));
        assert_eq!(screen.keys_view, a, "never current: the view as it stands");
        screen.keys_view = b;
        screen.keys_for(Some(1));
        assert_eq!(screen.keys_view, a);
        screen.keys_for(Some(1));
        assert_eq!(screen.keys_view, a, "still current: nothing moves");
        screen.keys_for(Some(2));
        assert_eq!(screen.keys_view, b);
    }

    #[test]
    fn icons_follow_the_kind() {
        let icon = |kind: ChannelKind| kind_icon(kind) as u8;
        assert_eq!(icon(ChannelKind::Master), 0);
        assert_eq!(icon(ChannelKind::Drums), 1);
        assert_eq!(icon(ChannelKind::Instrument), 2);
        assert_eq!(icon(ChannelKind::Rack), 2);
        assert_eq!(icon(ChannelKind::Synth(SynthKind::SuperSaw)), 2);
        assert_eq!(icon(ChannelKind::AudioClip), 21);
        assert_eq!(icon(ChannelKind::AudioRecord), 22);
        assert_eq!(icon(ChannelKind::EffectTrack), 44);
    }

    #[test]
    fn the_right_group_keeps_every_slot_while_all_fit_and_drops_the_meters_then_preview_then_redo()
    {
        let all = [true; 3];
        // n ≥ s: each keeps its slot, shown or not, so none moves under the finger.
        let kept = [Some(0), Some(1), Some(2), Some(3), Some(4)];
        assert_eq!(right_group(6, true, all), kept);
        assert_eq!(right_group(9, true, [false; 3]), kept);
        assert_eq!(
            right_group(3, false, [false; 3]),
            [None, None, Some(0), Some(1), Some(2)]
        );
        // n = 5 < s = 6, m = 6: RAM goes (m = 5), and then CPU fits.
        assert_eq!(
            right_group(5, true, all),
            [None, Some(0), Some(1), Some(2), Some(3)]
        );
        // n = 4: both meters go (m = 4), the three buttons keep theirs.
        assert_eq!(
            right_group(4, true, all),
            [None, None, Some(0), Some(1), Some(2)]
        );
        // n = 3: Preview goes too (m = 4 > 3, then 3), and Redo fits.
        assert_eq!(
            right_group(3, true, all),
            [None, None, None, Some(0), Some(1)]
        );
        // n = 2: only Undo, which always takes the next slot.
        assert_eq!(right_group(2, true, all), [None, None, None, None, Some(0)]);
        // A hidden Preview never keeps a short slot; with it gone CPU fits at n = 4 (m = 5, then 4).
        assert_eq!(
            right_group(4, true, [true, false, true]),
            [None, Some(0), None, Some(1), Some(2)]
        );
        // Meters off, n = 2 < s = 3: Preview goes (m = 3 > 2), Redo stays (m = 2).
        assert_eq!(
            right_group(2, false, all),
            [None, None, None, Some(0), Some(1)]
        );
    }

    #[test]
    fn the_transport_slots_are_b_squares_play_centred_and_the_close_x_moves_in_under_a_high_top() {
        for ppp in [1.0, 2.0] {
            let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX);
            let bar = Regions::new(crate::ui::offscreen::SCREEN, &m).transport;
            let (b, c, right) = (m.u(1.75), bar.center().x, bar.right());
            let sq = |left: f32| Rect::from_min_size(pos2(left, bar.top()), Vec2::splat(b));
            let near = |a: Rect, e: Rect, what: &str| {
                assert!(
                    (a.min - e.min).length() < 1e-3 && (a.max - e.max).length() < 1e-3,
                    "{what} at ppp {ppp}: {a:?} vs {e:?}"
                );
            };
            let s = transport_slots(bar, bar.top(), true, [true; 3], &m);
            near(s.play, sq(c - b / 2.0), "Play");
            near(s.rev, sq(c - 1.5 * b), "REV");
            near(s.rec, sq(c - 2.5 * b), "REC");
            near(s.bpm, sq(c + b / 2.0), "BPM");
            near(s.ctrl, sq(c + 1.5 * b), "CTRL");
            near(s.left[0], sq(bar.left()), "Mixer");
            near(s.left[1], sq(bar.left() + b), "Settings");
            near(s.lock, sq(bar.left() + 2.0 * b), "Lock");
            near(s.close, sq(right - b), "close");
            let from_close = |k: f32| sq(right - (k + 2.0) * b);
            near(s.ram.unwrap(), from_close(0.0), "RAM");
            near(s.cpu.unwrap(), from_close(1.0), "CPU");
            near(s.preview.unwrap(), from_close(2.0), "Preview");
            near(s.redo.unwrap(), from_close(3.0), "Redo");
            near(s.undo, from_close(4.0), "Undo");
            // A bar whose top is under 1.5 U from the screen's: the close X 3.75 U further left. The group follows it,
            // in 4 slots now: the meters go.
            let high = transport_slots(bar, m.u(1.4), true, [true; 3], &m);
            near(high.close, sq(right - m.u(3.75) - b), "close, high");
            assert_eq!((high.ram, high.cpu), (None, None));
            near(high.undo, sq(right - m.u(3.75) - 4.0 * b), "Undo, high");
            assert_eq!(high.play, s.play);
        }
    }

    #[test]
    fn bands_stack_ruler_tracks_transport_keyboard() {
        // A 5 U panel (160 points at U = 32): the transport bar 1.75 U, the keys the rest.
        let b = band_layout(540.0, 32.0, 160.0);
        assert_eq!(b.ruler, Rangef::new(0.0, 1.5 * 32.0));
        assert_eq!(b.transport, Rangef::new(380.0, 380.0 + 1.75 * 32.0));
        assert_eq!(b.keyboard, Rangef::new(436.0, 540.0));
        assert_eq!(b.tracks, Rangef::new(b.ruler.max, b.transport.min));
    }

    #[test]
    fn the_panel_is_its_preferred_height_in_u_by_default_5_u() {
        // `preferredKbdSize`, 5 U by default; it replaces the old quarter-of-the-screen measurement, so the whole panel
        // follows the UI unit.
        for (scale, u) in [(1, 32.0), (3, 48.0)] {
            let m = Metrics::from_pixels_per_point(1.0, scale);
            assert_eq!(m.u(1.0), u);
            let r = Regions::new(crate::ui::offscreen::SCREEN, &m);
            assert_eq!(r.transport.top(), 540.0 - 5.0 * u, "U {u}");
            assert_eq!(r.transport.height(), 1.75 * u);
            assert_eq!(
                (r.keyboard.top(), r.keyboard.bottom()),
                (r.transport.bottom(), 540.0)
            );
            assert_eq!(r.area.bottom(), r.transport.top());
            // Sized: any height, the screen's at most.
            let tall = Regions::sized(crate::ui::offscreen::SCREEN, 8.0 * u, &m);
            assert_eq!(tall.transport.top(), 540.0 - 8.0 * u);
            assert_eq!(tall.keyboard.height(), 6.25 * u);
        }
    }

    #[test]
    fn the_add_button_sits_below_the_last_row_on_the_screen_centre() {
        let m = Metrics::from_pixels_per_point(1.0, 1); // U = 32
        // A playlist narrower than the screen (as with the rack open): the screen's centre counts.
        let screen = Rect::from_x_y_ranges(0.0..=1170.0, 0.0..=540.0);
        let tracks = Regions::new(screen, &m).area.with_min_y(48.0);
        let area = tracks.with_max_x(585.0);
        let at = |rows, bottom| add_button_center(screen, area, rows, bottom, &m);
        // Empty song: midway between the master row (bottom 112) and the area's bottom.
        assert_eq!(at(1, 112.0), pos2(585.0, (112.0 + area.bottom()) / 2.0));
        // Otherwise 0.75 U below the last row: its centre 1.75 U down.
        assert_eq!(at(2, 176.0), pos2(585.0, 232.0));
        assert_eq!(at(3, 240.0), pos2(585.0, 296.0));
        // Past the area's bottom, it scrolls with the rows.
        assert_eq!(at(6, 432.0), pos2(585.0, 488.0));
    }

    #[test]
    fn the_empty_song_plus_is_two_hairline_bars_spanning_about_half_the_disc() {
        // UI scale 100 % and 110 % at ppp 1, 2 and 1.5: U is 32, 64, 35, 70, 48 and 53 px, t 2, 4, 3, 5, 3 and 4 px.
        for (ppp, scale) in [(1.0, 1), (2.0, 1), (1.0, 5), (2.0, 5), (1.5, 1), (1.5, 5)] {
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let what = format!("ppp {ppp}, U {}", m.u_px());
            let center = pos2(500.3, 230.5);
            // The two bars the screen paints over the disc (this checks the helper, not the painted shapes).
            let [h, v] = plus_bars(center, &m);
            // Whole device pixels divided by a fractional ppp are not exact in f32: compare those to 0.001 px.
            let eps = if ppp.fract() == 0.0 { 0.0 } else { 1e-3 };
            let near = |a: f32, b: f32| (a - b).abs() * ppp <= eps;
            assert!(
                near(h.height(), m.t()) && near(v.width(), m.t()),
                "{what}: {h:?} {v:?}"
            );
            assert!(near(h.width(), v.height()), "{what}: {h:?} {v:?}");
            let px = 1.0 / ppp;
            assert!((h.width() - m.u(1.12)).abs() <= px, "{what}: {h:?}");
            // A symmetric cross on whole pixels, within half a pixel of the disc centre.
            assert!((h.center() - v.center()).length() * ppp <= eps, "{what}");
            assert!((h.center() - center).abs().max_elem() <= px / 2.0, "{what}");
            for edge in [h.min, h.max, v.min, v.max] {
                let d = edge * ppp;
                assert!((d - d.round()).abs().max_elem() <= eps, "{what}: {edge:?}");
            }
        }
    }

    #[test]
    fn the_rows_scroll_over_every_row_and_the_room_for_the_plus() {
        // 9 rows of 64 plus 112 for the `+` (0.75 U, 2 U, 0.75 U) = 688 in a 301-tall area: 387 to scroll.
        let m = Metrics::from_pixels_per_point(1.0, 1);
        assert_eq!(content_height(9, m.u(ROW_H), &m), 688.0);
        let zoom = 1.0 / 64.0;
        assert_eq!(rows_range(9, zoom, &m), 10.75);
        let y = |rows| Axis {
            pos: 0.0,
            zoom,
            range: [0.0, rows_range(rows, zoom, &m)],
            zoom_range: [zoom; 2],
            view: Rangef::new(0.0, 301.0),
        };
        assert_eq!(y(9).max() * 64.0, 387.0);
        // Content that fits never scrolls.
        assert_eq!(y(1).max(), 0.0);
    }

    #[test]
    fn only_rows_touching_the_view_are_visible() {
        // 64-tall rows in a 301-tall view.
        assert_eq!(visible_rows(0.0, 301.0, 64.0, 9), 0..5);
        assert_eq!(visible_rows(32.0, 301.0, 64.0, 9), 0..6);
        assert_eq!(visible_rows(64.0, 301.0, 64.0, 9), 1..6);
        assert_eq!(visible_rows(371.0, 301.0, 64.0, 9), 5..9);
        // Fewer rows than the view holds, and no rows at all.
        assert_eq!(visible_rows(0.0, 301.0, 64.0, 2), 0..2);
        assert_eq!(visible_rows(0.0, 301.0, 64.0, 0), 0..0);
        // Scrolled past everything (cannot happen once clamped): empty, not out of bounds.
        assert_eq!(visible_rows(1000.0, 301.0, 64.0, 9), 9..9);
    }

    /// What one [`frame`] reports: the metrics, the track area, the song edit and the navigation.
    struct Frame {
        m: Metrics,
        area: Rect,
        edit: Option<SongEdit>,
        nav: Option<Nav>,
        /// The song tick a ruler tap asked the playhead to go to.
        playhead: Option<u64>,
        /// The track menu's pick, and the insert popup's.
        track: Option<(usize, usize, TrackOp)>,
        insert: Option<(usize, ChannelKind)>,
        reorder: Option<Vec<usize>>,
        /// The playhead knob's drop: the song tick it seeks to.
        seek: Option<u64>,
        /// Where the track grid starts.
        grid_left: f32,
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    /// One frame of the playlist and the chrome for `song` in an offscreen context (1170 x 540 points, 1 pixel
    /// per point) at the UI-scale index `scale`. `selected` is the keyboard's channel; Preview plays `preview` alone.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        ctx: &egui::Context,
        screen: &mut MainScreen,
        song: &mut Song,
        time: &mut f64,
        scale: usize,
        selected: &mut usize,
        preview: Option<ClipKey>,
        events: Vec<egui::Event>,
    ) -> Frame {
        *time += 0.1;
        let size = vec2(1170.0, 540.0);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            time: Some(*time),
            ..Default::default()
        };
        let theme = theme_by_id(0);
        let tracks = track_rows(song);
        let follow = FOLLOW.get();
        let output_snapshot = follow.map(|tick| OutputSnapshot {
            transport_tick: tick,
            ..snapshot(0.0, false)
        });
        let settings = Settings {
            continuous_scroll: follow.is_some(),
            ..Settings::default()
        };
        let mut audio = AudioLink::default();
        let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        USER_FILES.with_borrow(|user| {
            for (path, bytes) in user {
                files.keep_user_file(std::path::Path::new(path), bytes.clone());
            }
        });
        let mut result = None;
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let m = Metrics::new(ctx, scale);
                    let cx = UiCtx {
                        theme: &theme,
                        m: &m,
                    };
                    let input = ScreenInput {
                        view: View::Playlist,
                        tracks: &tracks,
                        selected: 0,
                        settings: &settings,
                        audio: output_snapshot.as_ref(),
                        bpm: 140.0,
                        history: (false, false),
                        preview,
                        can_preview: false,
                        sample_row: None,
                        pads: false,
                        keyboard: Default::default(),
                        piano: true,
                        covered: false,
                        wheels: [0.0, 0.5],
                        chords: None,
                    };
                    let regions = Regions::new(ui.max_rect(), &m);
                    let mut app = AppCtx {
                        cx,
                        regions,
                        song: &mut *song,
                        audio: &mut audio,
                        selected: &mut *selected,
                        snapshot: None,
                        playing: false,
                        armed: false,
                        previewing: false,
                        middle_c_is_c4: false,
                        settings: &mut Settings::default(),
                        ghost_notes: true,
                        files: &mut files,
                        edit: None,
                        answer: None,
                        covered: false,
                        playhead: None,
                        seek: None,
                        midi_devices: &[],
                        midi_rescan: false,
                    };
                    let mut actions = ScreenActions::default();
                    screen.playlist(ui, &mut app, input, &mut actions);
                    let (edit, playhead) = (app.edit, app.playhead);
                    screen.chrome(ui, &cx, input, &regions, &mut actions);
                    screen.corner_buttons(ui, &cx, input.selected, &regions, &mut actions);
                    let panel_h = size.y - regions.transport.top();
                    let tracks = band_layout(size.y, m.u(1.0), panel_h).tracks;
                    result = Some(Frame {
                        m,
                        area: Rect::from_x_y_ranges(0.0..=size.x, tracks.min..=tracks.max),
                        edit,
                        nav: actions.nav,
                        playhead,
                        track: actions.track,
                        insert: actions.insert,
                        reorder: actions.reorder,
                        seek: actions.seek,
                        grid_left: screen.header_w(&m) + m.t(),
                        shapes: Vec::new(),
                    });
                });
        });
        let mut result = result.unwrap();
        result.shapes = output.shapes;
        result
    }

    /// One [`frame`] at UI scale 100 %; returns the metrics and the track area.
    fn run_frame(
        ctx: &egui::Context,
        screen: &mut MainScreen,
        song: &mut Song,
        time: &mut f64,
        events: Vec<egui::Event>,
    ) -> (Metrics, Rect) {
        run_frame_at(ctx, screen, song, time, 1, events)
    }

    /// [`run_frame`] at the UI-scale index `scale`.
    fn run_frame_at(
        ctx: &egui::Context,
        screen: &mut MainScreen,
        song: &mut Song,
        time: &mut f64,
        scale: usize,
        events: Vec<egui::Event>,
    ) -> (Metrics, Rect) {
        let f = frame(ctx, screen, song, time, scale, &mut 0, None, events);
        (f.m, f.area)
    }

    fn song_with_channels(n: usize) -> Song {
        let mut song = Song::default();
        for _ in 0..n {
            song.add_channel(ChannelKind::Drums);
        }
        song
    }

    fn wheel(dy: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            modifiers: Default::default(),
        }
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn wheel_scrolls_eight_channels_until_the_last_row_fits_the_track_area() {
        let ctx = egui::Context::default();
        let (mut screen, mut time) = (MainScreen::default(), 0.0);
        let mut song = song_with_channels(8); // master + 8 = 9 rows
        let (mut m, mut area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        let last = song.channels.len() - 1;
        // Not scrolled: the last row is far below the area.
        assert!(row_top(area, screen.scroll_y, last, m.u(ROW_H)) > area.bottom());

        let over_rows = area.center();
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(over_rows), wheel(-5000.0)],
        );
        for _ in 0..60 {
            (m, area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        }
        let max = content_height(song.channels.len(), m.u(ROW_H), &m) - area.height();
        assert_eq!(screen.scroll_y, max);
        let top = row_top(area, screen.scroll_y, last, m.u(ROW_H));
        let last_row = Rect::from_x_y_ranges(area.x_range(), top..=top + m.u(ROW_H));
        assert!(
            area.contains_rect(last_row),
            "{last_row:?} not inside {area:?}"
        );
        // The `+` is 0.75 U below it, and 0.75 U above the area's bottom.
        let plus = add_button_center(area, area, song.channels.len(), last_row.bottom(), &m);
        assert_eq!(plus.y - m.u(ADD_D) / 2.0, last_row.bottom() + m.u(0.75));
        assert_eq!(plus.y + m.u(ADD_D) / 2.0, area.bottom() - m.u(0.75));

        // Wheeling back up returns to the top.
        run_frame(&ctx, &mut screen, &mut song, &mut time, vec![wheel(5000.0)]);
        for _ in 0..60 {
            run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        }
        assert_eq!(screen.scroll_y, 0.0);
    }

    #[test]
    fn wheel_outside_the_track_area_does_not_scroll() {
        let ctx = egui::Context::default();
        let (mut screen, mut time) = (MainScreen::default(), 0.0);
        let mut song = song_with_channels(8);
        let (_, area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        let over_keyboard = pos2(300.0, area.bottom() + 150.0);
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(over_keyboard), wheel(-500.0)],
        );
        for _ in 0..30 {
            run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        }
        assert_eq!(screen.scroll_y, 0.0);
    }

    #[test]
    fn vertical_drag_scrolls_but_not_when_it_starts_on_the_add_button() {
        let ctx = egui::Context::default();
        let (mut screen, mut time) = (MainScreen::default(), 0.0);
        let mut song = song_with_channels(8);
        let (_, area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);

        // Drag up by 100 points over the rows: the content follows the pointer.
        let from = pos2(300.0, area.center().y);
        let to = from - vec2(0.0, 100.0);
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(from)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![button(from, true)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(to)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![button(to, false)],
        );
        assert_eq!(screen.scroll_y, 100.0);
        // The release flings (the last move was 100 points); let it run out.
        for _ in 0..200 {
            run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        }

        // Scroll to the end, then drag down starting on the `+`: nothing moves.
        screen.scroll_y = f32::MAX;
        let (m, area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        let end = screen.scroll_y;
        let rows_bottom = row_top(area, end, song.channels.len(), m.u(ROW_H));
        let plus = add_button_center(area, area, song.channels.len(), rows_bottom, &m);
        let to = plus + vec2(0.0, 40.0);
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(plus)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![button(plus, true)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![egui::Event::PointerMoved(to)],
        );
        run_frame(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            vec![button(to, false)],
        );
        assert_eq!(screen.scroll_y, end);
    }

    fn snapshot(cpu_load: f32, stream_failed: bool) -> OutputSnapshot {
        OutputSnapshot {
            transport_tick: 0,
            cpu_load,
            module_meters: Vec::new(),
            stream_failed,
            ..OutputSnapshot::default()
        }
    }

    #[test]
    fn the_cpu_bar_follows_the_load_clamped_and_is_empty_without_a_working_stream() {
        let share = |load, failed| cpu_share(Some(&snapshot(load, failed)));
        assert_eq!(share(0.4, false), 0.4);
        assert_eq!(share(1.7, false), 1.0);
        assert_eq!(share(-0.1, false), 0.0);
        assert_eq!(share(f32::NAN, false), 0.0);
        assert_eq!(share(0.4, true), 0.0);
        assert_eq!(cpu_share(None), 0.0);
    }

    /// An offscreen rig and a painter of the playlist's transport bar (the default panel's) that asserts, every frame, that
    /// it neither rewinds nor navigates; `f` gets them with REC's slot and the arm as the last frame left it.
    fn with_transport(
        f: impl FnOnce(
            &mut Offscreen,
            &mut dyn FnMut(&mut egui::Ui, &UiCtx),
            Rect,
            &std::cell::Cell<bool>,
        ),
    ) {
        let (tracks, settings) = (track_rows(&Song::default()), Settings::default());
        let input = ScreenInput {
            view: View::Playlist,
            tracks: &tracks,
            selected: 0,
            settings: &settings,
            audio: None,
            bpm: 140.0,
            history: (false, false),
            preview: None,
            can_preview: false,
            sample_row: None,
            pads: false,
            keyboard: Default::default(),
            piano: true,
            covered: false,
            wheels: [0.0, 0.5],
            chords: None,
        };
        let regions = Regions::new(
            crate::ui::offscreen::SCREEN,
            &crate::ui::offscreen::metrics(),
        );
        let bar = regions.transport;
        let mut screen = MainScreen::default();
        let armed = std::cell::Cell::new(false);
        let mut paint = |ui: &mut egui::Ui, cx: &UiCtx| {
            let mut actions = ScreenActions::default();
            screen.transport(ui, &regions, cx, input, &mut actions);
            assert!(!actions.rewind && actions.nav.is_none());
            armed.set(screen.armed);
        };
        let rec = transport_slot(bar, 0.0, &crate::ui::offscreen::metrics());
        f(&mut Offscreen::new(), &mut paint, rec, &armed);
    }

    #[test]
    fn rec_toggles_the_arm_and_draws_its_icon_in_the_record_colour_while_armed() {
        with_transport(|rig, paint, rec, armed| {
            let text = theme_by_id(0).colors.text.to_color32();
            let ink = |shapes: &[egui::epaint::ClippedShape]| {
                let caption = shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "REC" => {
                        Some(t.galley.rows[0].visuals.mesh.vertices[0].color)
                    }
                    _ => None,
                });
                let disc = shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if rec.contains(c.center) => Some(c.fill),
                    _ => None,
                });
                (caption, disc)
            };
            let idle = rig.frame(vec![], paint);
            assert_eq!(ink(&idle), (Some(text), Some(text)));
            assert!(!armed.get());
            // A tap arms: the icon (a filled circle) turns the record colour from that frame on; the caption stays.
            let at = rec.center();
            rig.frame(vec![egui::Event::PointerMoved(at)], paint);
            rig.frame(vec![button(at, true)], paint);
            let tapped = rig.frame(vec![button(at, false)], paint);
            assert!(armed.get());
            let red = crate::theme::RECORD_COLOR.to_color32();
            assert_eq!(ink(&tapped), (Some(text), Some(red)));
            // Another tap disarms.
            rig.frame(vec![button(at, true)], paint);
            let tapped = rig.frame(vec![button(at, false)], paint);
            assert!(!armed.get());
            assert_eq!(ink(&tapped), (Some(text), Some(text)));
        });
    }

    #[test]
    fn a_tap_just_inside_recs_right_edge_arms_and_does_not_rewind() {
        // REV's slot starts at REC's right edge, within egui's 5 px snap radius of the tap.
        with_transport(|rig, paint, rec, armed| {
            let at = pos2(rec.right() - 2.0, rec.center().y);
            rig.frame(vec![egui::Event::PointerMoved(at)], paint);
            rig.frame(vec![button(at, true)], paint);
            // `paint` asserts that no frame rewinds.
            rig.frame(vec![button(at, false)], paint);
            assert!(armed.get());
        });
    }

    #[test]
    fn rec_tells_accessibility_tools_it_is_the_arm_recording_toggle() {
        use egui::accesskit::{NodeId, Role, Toggled};
        let (tracks, settings) = (track_rows(&Song::default()), Settings::default());
        let input = ScreenInput {
            view: View::Playlist,
            tracks: &tracks,
            selected: 0,
            settings: &settings,
            audio: None,
            bpm: 140.0,
            history: (false, false),
            preview: None,
            can_preview: false,
            sample_row: None,
            pads: false,
            keyboard: Default::default(),
            piano: true,
            covered: false,
            wheels: [0.0, 0.5],
            chords: None,
        };
        let (theme, m) = (theme_by_id(0), crate::ui::offscreen::metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for armed in [false, true] {
            let mut screen = MainScreen {
                armed,
                ..MainScreen::default()
            };
            let raw = egui::RawInput {
                screen_rect: Some(crate::ui::offscreen::SCREEN),
                ..Default::default()
            };
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let regions = Regions::new(crate::ui::offscreen::SCREEN, &m);
                    let mut actions = ScreenActions::default();
                    screen.transport(ui, &regions, &cx, input, &mut actions);
                });
            });
            let update = output
                .platform_output
                .accesskit_update
                .expect("accesskit on");
            let id = Id::new(("transport", "REC"));
            let node = update.nodes.iter().find(|(n, _)| *n == NodeId(id.value()));
            let node = node.map(|(_, node)| node).expect("a REC node");
            assert_eq!(node.role(), Role::CheckBox);
            assert_eq!(node.label(), Some("Arm recording"));
            let toggled = if armed { Toggled::True } else { Toggled::False };
            assert_eq!(node.toggled(), Some(toggled));
        }
    }

    /// Moves to `pos`, presses and releases the primary button, one frame each.
    fn click(
        ctx: &egui::Context,
        screen: &mut MainScreen,
        song: &mut Song,
        time: &mut f64,
        scale: usize,
        pos: Pos2,
    ) {
        for events in [
            vec![egui::Event::PointerMoved(pos)],
            vec![button(pos, true)],
            vec![button(pos, false)],
        ] {
            run_frame_at(ctx, screen, song, time, scale, events);
        }
    }

    #[test]
    fn a_click_outside_the_open_popup_closes_it_without_reaching_the_widget_below() {
        // Scale 60 %: the popup is small enough to leave the play button uncovered.
        let scale = 8;
        let ctx = egui::Context::default();
        let (mut screen, mut time) = (MainScreen::default(), 0.0);
        let mut song = song_with_channels(0);
        for _ in 0..3 {
            run_frame_at(&ctx, &mut screen, &mut song, &mut time, scale, vec![]);
        }
        let (m, area) = run_frame_at(&ctx, &mut screen, &mut song, &mut time, scale, vec![]);
        let plus = add_button_center(area, area, 1, row_top(area, 0.0, 1, m.u(ROW_H)), &m);
        click(&ctx, &mut screen, &mut song, &mut time, scale, plus);
        assert!(screen.add_channel_open, "the + did not open the popup");
        // The click that opened it does not close it.
        for _ in 0..3 {
            run_frame_at(&ctx, &mut screen, &mut song, &mut time, scale, vec![]);
        }
        assert!(screen.add_channel_open, "the popup closed by itself");

        let transport = Regions::new(crate::ui::offscreen::SCREEN, &m).transport;
        let play = transport.center();
        click(&ctx, &mut screen, &mut song, &mut time, scale, play);
        assert!(!screen.add_channel_open, "the outside click did not close");
        assert!(!screen.playing, "the outside click toggled play");

        // With the popup gone, the same click does reach the play button.
        click(&ctx, &mut screen, &mut song, &mut time, scale, play);
        assert!(screen.playing);
    }

    #[test]
    fn a_click_in_the_corner_of_the_plus_square_does_not_open_the_popup() {
        let ctx = egui::Context::default();
        let (mut screen, mut time) = (MainScreen::default(), 0.0);
        let mut song = song_with_channels(0);
        for _ in 0..3 {
            run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        }
        let (m, area) = run_frame(&ctx, &mut screen, &mut song, &mut time, vec![]);
        let plus = add_button_center(area, area, 1, row_top(area, 0.0, 1, m.u(ROW_H)), &m);
        let r = m.u(ADD_D) / 2.0;
        // Inside the bounding square, outside the disc.
        click(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            1,
            plus + vec2(0.9 * r, 0.9 * r),
        );
        assert!(!screen.add_channel_open);
        // Near the rim on the axis is still the disc.
        click(
            &ctx,
            &mut screen,
            &mut song,
            &mut time,
            1,
            plus + vec2(0.9 * r, 0.0),
        );
        assert!(screen.add_channel_open);
    }

    /// Master, Drums (1) and MiniSynth (2), with a one-bar clip of two notes on the MiniSynth at bar 2. At UI
    /// scale 100 % (U 32) a beat is 38.4 points from the grid's left edge at x 48, and track row `i` spans
    /// y 48 + 64 i ..; the clip spans x 201.6 ..= 355.2, y 176 ..= 239.
    fn song_with_a_clip() -> Song {
        let mut song = Song::default();
        song.add_channel(ChannelKind::Drums);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let note = |pos, key| Event::Note {
            pos,
            len_beats: 1.0,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        };
        song.channels[2].lanes[0].add_clip(512, 4.0).events = vec![note(0, 60), note(256, 67)];
        // The song ends at beat 64, off screen on the master's track: the time scrolls over 0 … 128 beats, and the
        // range/view cap leaves the rig's 38.4 points a beat.
        song.channels[0].lanes[0].add_clip(60 * PPQ, 4.0);
        song
    }

    /// Beat 6 of the MiniSynth's clip (beats 4 … 8 of track 2).
    const CLIP: Pos2 = pos2(278.4, 207.5);
    /// A beat's width in the rig: [`Rig::new`] stores it as the song's view (HEAD@296 zoom x, at ppp 1), which a song
    /// at least 15 beats long keeps (the range/view cap).
    const BEAT_PT: f32 = 38.4;
    /// The grid's left edge: the 1.5 U column and t = 2 points (U = 32).
    const GRID_X: f32 = 50.0;

    /// The playlist with its own song, for the clip tests.
    struct Rig {
        ctx: egui::Context,
        screen: MainScreen,
        song: Song,
        time: f64,
        /// The keyboard's channel.
        selected: usize,
        /// The clip Preview plays alone.
        preview: Option<ClipKey>,
    }

    impl Rig {
        fn new(mut song: Song) -> Self {
            if song.file.playlist_view[2] == 0.01 {
                song.file.playlist_view[2] = 1.0 / f64::from(BEAT_PT);
            }
            let mut rig = Self {
                ctx: egui::Context::default(),
                screen: MainScreen::default(),
                song,
                time: 0.0,
                selected: 1,
                preview: None,
            };
            rig.frame(vec![]);
            rig
        }

        /// The current clip: the current track's own.
        fn current_clip(&self) -> Option<ClipRef> {
            self.screen.clips.clip_for(&self.song, self.selected)
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> Frame {
            let Self {
                ctx,
                screen,
                song,
                time,
                selected,
                preview,
            } = self;
            frame(ctx, screen, song, time, 1, selected, *preview, events)
        }

        /// Runs a frame per event list; returns the strongest song edit and the last navigation.
        fn frames(&mut self, frames: Vec<Vec<egui::Event>>) -> (Option<SongEdit>, Option<Nav>) {
            let (mut edit, mut nav) = (None, None);
            for events in frames {
                let f = self.frame(events);
                edit = edit.max(f.edit);
                nav = f.nav.or(nav);
            }
            (edit, nav)
        }

        fn click(&mut self, pos: Pos2) -> (Option<SongEdit>, Option<Nav>) {
            self.frames(vec![
                vec![egui::Event::PointerMoved(pos)],
                vec![button(pos, true)],
                vec![button(pos, false)],
            ])
        }

        /// The rect of widget `id` as the last frame laid it out, after one more frame: egui reads widget rects
        /// from the frame before the one that just ran.
        fn rect_of(&mut self, id: Id) -> Option<Rect> {
            self.frame(vec![]);
            self.ctx.read_response(id).map(|r| r.rect)
        }

        fn click_id(&mut self, id: Id) -> (Option<SongEdit>, Option<Nav>) {
            let rect = self.rect_of(id).expect("the widget was drawn");
            self.click(rect.center())
        }

        /// Presses at `from`, moves to `to` in two steps and releases there. Returns each frame's edit.
        fn drag(&mut self, from: Pos2, to: Pos2) -> Vec<Option<SongEdit>> {
            let mid = from + (to - from) / 2.0;
            [
                vec![egui::Event::PointerMoved(from)],
                vec![button(from, true)],
                vec![egui::Event::PointerMoved(mid)],
                vec![egui::Event::PointerMoved(to)],
                vec![button(to, false)],
            ]
            .into_iter()
            .map(|events| self.frame(events).edit)
            .collect()
        }

        fn clips(&self, channel: usize) -> &[crate::song::Clip] {
            &self.song.channels[channel].lanes[0].clips
        }

        /// Taps the `+Clip` button showing on the current track.
        fn plus(&mut self) -> (Option<SongEdit>, Option<Nav>) {
            self.click_id(Id::new("clip_plus"))
        }

        /// A tap at `pos` 0.1 s long, then another 0.1 s later: released 0.2 s apart, a double tap.
        fn double_tap(&mut self, pos: Pos2) -> (Option<SongEdit>, Option<Nav>) {
            self.frames(vec![
                vec![egui::Event::PointerMoved(pos)],
                vec![button(pos, true)],
                vec![button(pos, false)],
                vec![button(pos, true)],
                vec![button(pos, false)],
            ])
        }

        /// A press at `pos` held 0.4 s, then released there: a long press.
        fn long_press(&mut self, pos: Pos2) -> (Option<SongEdit>, Option<Nav>) {
            self.frames(vec![
                vec![egui::Event::PointerMoved(pos)],
                vec![button(pos, true)],
                vec![],
                vec![],
                vec![],
                vec![button(pos, false)],
            ])
        }

        /// The `+Clip` button is drawn, a frame on.
        fn plus_shown(&mut self) -> bool {
            let f = self.frame(vec![]);
            (f.shapes.iter())
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "+Clip"))
        }

        /// Taps the MiniSynth's clip and lets the double-tap window pass: current and selected, it drags at once.
        fn current(&mut self) {
            self.click(CLIP);
            self.frame(vec![]);
            self.frame(vec![]);
        }
    }

    /// The MiniSynth's clip of [`song_with_a_clip`].
    const SYNTH_CLIP: ClipRef = ClipRef {
        channel: 2,
        lane: 0,
        id: 1,
    };

    /// On the ruler, over beat `b`.
    fn ruler(b: f32) -> Pos2 {
        pos2(GRID_X + b * BEAT_PT, 24.0)
    }

    impl Rig {
        /// A long press on the ruler over beat `from` (held 0.3 s), dragged to `to` and released: the range.
        fn select(&mut self, from: f32, to: f32) {
            self.frame(vec![egui::Event::PointerMoved(ruler(from))]);
            self.frame(vec![button(ruler(from), true)]);
            self.time += 0.21;
            self.frame(vec![]);
            self.frame(vec![egui::Event::PointerMoved(ruler(to))]);
            self.frame(vec![button(ruler(to), false)]);
        }

        /// Selects beats `from..to` and taps the `+` under the released edge: the region menu opens.
        fn region(&mut self, from: f32, to: f32) {
            self.select(from, to);
            self.click_id(Id::new("playlist_region").with("plus"));
        }

        /// Clicks the region menu's item in `row`, found from row 0 (a disabled item takes no input, so egui keeps
        /// no rect for it); returns the strongest song edit.
        fn region_item(&mut self, row: usize) -> Option<SongEdit> {
            let first = Id::new("playlist_region")
                .with("menu")
                .with((0_usize, 0_usize));
            let first = self.rect_of(first).expect("the region menu is open");
            self.click(first.center() + vec2(0.0, row as f32 * first.height()))
                .0
        }
    }

    /// Every play of channel `channel`'s main-lane clips as (song tick, key), in time order.
    fn plays(song: &Song, channel: usize) -> Vec<(u64, u8)> {
        let mut plays: Vec<_> = (song.channels[channel].lanes[0].clips.iter())
            .flat_map(|c| {
                c.events_in_song_range(0, 100_000).map(|(t, e)| match *e {
                    Event::Note { key, .. } => (t, key),
                    _ => (t, 0),
                })
            })
            .collect();
        plays.sort();
        plays
    }

    #[test]
    fn the_playlists_bars_are_tdiv_byte_1_beats() {
        // TDIV: byte 1 = beats per bar b, byte 0 = the subdivision.
        for (d, b) in [(4u8, 3u8), (3, 7)] {
            let mut song = song_with_a_clip();
            song.time_sig = TimeSig {
                subdivision: d,
                beats_per_bar: b,
            };
            let mut rig = Rig::new(song);
            let f = rig.frame(vec![]);
            let bar_x = |n: f32| GRID_X + n * f32::from(b) * BEAT_PT;
            // The ruler's bar-3 badge two bars right of the grid's left edge (x 48).
            let badges: Vec<f32> = (f.shapes.iter())
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == "3" && t.pos.y < 48.0 => {
                        Some(t.pos.x + t.galley.rect.center().x)
                    }
                    _ => None,
                })
                .collect();
            assert!(
                badges.iter().any(|&x| (x - bar_x(2.0)).abs() < 0.5),
                "b {b}: {badges:?}"
            );
            // The rows' 4 px bar lines (thinner beat lines between), every b beats, on the pixel grid.
            let lines: Vec<f32> = (f.shapes.iter())
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if (r.rect.width() - 4.0).abs() < 1e-3 && r.rect.top() >= 48.0 =>
                    {
                        Some(r.rect.center().x)
                    }
                    _ => None,
                })
                .collect();
            assert!(
                lines.iter().any(|&x| (x - bar_x(1.0)).abs() <= 1.0),
                "b {b}: {lines:?}"
            );
            assert!(
                !lines
                    .iter()
                    .any(|&x| (x - GRID_X - 4.0 * BEAT_PT).abs() <= 1.0),
                "b {b}: {lines:?}"
            );
            // A tap on the empty current track shows `+Clip`, which adds a one-bar clip: b beats.
            let drums = pos2(GRID_X + 0.3 * BEAT_PT, 144.0);
            rig.click(drums);
            rig.plus();
            assert_eq!(rig.clips(1)[0].loop_len_beats, f64::from(b));
        }
    }

    #[test]
    fn a_take_being_recorded_is_a_two_tone_bar_on_its_track() {
        // Two beats on the Drums row (y 112 ..= 175), from the grid's left edge.
        let mut rig = Rig::new(song_with_a_clip());
        let take = |channel| LiveTake {
            channel,
            lane: 0,
            start: 0.0,
            end: 256.0,
            clip: None,
        };
        rig.screen.takes = vec![take(1), take(7)];
        let f = rig.frame(vec![]);
        let theme = theme_by_id(0);
        let rects: Vec<(Rect, Color32)> = (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.x_range() == Rangef::new(GRID_X, 126.0) => {
                    Some((r.rect, r.fill))
                }
                _ => None,
            })
            .collect();
        let color = channel_color(rig.song.channels[1].hue, &theme).to_color32();
        // A recording clip: its clip box is x 50 … 126 (2 beats' x, 126.8, truncated), y 113 … 173
        // (Y_r + 1 … Y_{r+1} − t − 1); a channel-colour box a pixel in from its top and bottom, then a content-colour
        // one 3 px in. A take on a channel that does not exist draws nothing.
        let [(outer, outer_fill), (inner, inner_fill)] = rects[..] else {
            panic!("{rects:?}")
        };
        let rect = |y0: f32, y1: f32| Rect::from_min_max(pos2(GRID_X, y0), pos2(126.0, y1));
        assert_eq!((outer, outer_fill), (rect(114.0, 172.0), color));
        assert_eq!(inner, rect(116.0, 170.0));
        assert_ne!(inner_fill, outer_fill);
    }

    // ---- the rows and clips as the playlist draws them ----

    /// Row `i`'s fill right of the headers (x [`GRID_X`]): y 48 + 64 i, 63 tall (a pixel of gap under it).
    fn row_fills(f: &Frame, rows: usize) -> Vec<Color32> {
        (0..rows)
            .filter_map(|i| {
                let top = 48.0 + 64.0 * i as f32;
                f.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.rect.left() == GRID_X
                            && r.rect.top() == top
                            && r.rect.height() == 63.0 =>
                    {
                        Some(r.fill)
                    }
                    _ => None,
                })
            })
            .collect()
    }

    #[test]
    fn the_current_tracks_row_is_module_fill_once_there_are_two_channels() {
        let c = theme_by_id(0).colors;
        let (module, control) = (c.module_fill.to_color32(), c.control_fill.to_color32());
        // The keyboard's channel, Drums (row 1), has the current track.
        let mut rig = Rig::new(song_with_a_clip());
        assert_eq!(row_fills(&rig.frame(vec![]), 3), [control, module, control]);
        // A tap on the MiniSynth's clip makes its track current.
        rig.click(CLIP);
        assert_eq!(row_fills(&rig.frame(vec![]), 3), [control, control, module]);
        // The master alone, its track current: fewer than 2 channels, every row `controlFill`.
        let mut rig = Rig::new(Song::default());
        rig.selected = 0;
        assert_eq!(row_fills(&rig.frame(vec![]), 1), [control]);
        // The master and one instrument: two channels, the current track lit.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let mut rig = Rig::new(song);
        assert_eq!(row_fills(&rig.frame(vec![]), 2), [control, module]);
    }

    #[test]
    fn the_rows_beat_lines_are_background_4_2_1_px_centred_on_marks_at_least_0_6_u_apart() {
        // 38.4 points a beat at U 32: half beats are 19.2 apart, 0.6 U, and quarters would be closer. Bars 4 px, beats
        // 2 px, half beats 1 px, each ⌊w/2⌋ px left of its mark rounded to the pixel grid. Drums' row (y 112, 63 tall).
        let mut rig = Rig::new(song_with_a_clip());
        let f = rig.frame(vec![]);
        let bg = theme_by_id(0).colors.background.to_color32();
        let mut lines: Vec<(f32, f32)> = (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.rect.top() == 112.0 && r.rect.height() == 63.0 && r.fill == bg =>
                {
                    Some((r.rect.left(), r.rect.width()))
                }
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let want = [
            (48.0, 4.0),
            (69.0, 1.0),
            (87.0, 2.0),
            (108.0, 1.0),
            (126.0, 2.0),
            (146.0, 1.0),
            (164.0, 2.0),
            (184.0, 1.0),
            (202.0, 4.0),
        ];
        assert_eq!(lines[..9], want);
        // At ppp 2 the same marks in half-point pixels: w px wide, ⌊w/2⌋ px left of the mark on the half-point grid.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        let theme = theme_by_id(0);
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let song = song_with_a_clip();
        let tracks: Vec<(TrackRow, f32)> = (track_rows(&song).into_iter())
            .zip([48.0, 112.0, 176.0])
            .collect();
        let axis = TimeAxis {
            origin: 48.0,
            beat_w: BEAT_PT,
        };
        let rect = Rect::from_min_max(pos2(0.0, 48.0), pos2(1170.0, 380.0));
        let rows = (TimeSig::default(), 64.0, None);
        let out = egui::Context::default().run(egui::RawInput::default(), |ctx| {
            let panel = egui::CentralPanel::default().frame(egui::Frame::NONE);
            panel.show(ctx, |ui| {
                paint_track_rows(ui.painter(), rect, 48.0, &tracks, axis, rows, &cx);
            });
        });
        let mut lines: Vec<(f32, f32)> = (out.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.rect.top() == 112.0 && r.rect.height() == 63.5 && r.fill == bg =>
                {
                    Some((r.rect.left(), r.rect.width()))
                }
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.total_cmp(&b.0));
        let want = [
            (47.0, 2.0),
            (67.0, 0.5),
            (86.0, 1.0),
            (105.5, 0.5),
            (124.5, 1.0),
            (144.0, 0.5),
            (162.5, 1.0),
            (182.5, 0.5),
            (200.5, 2.0),
        ];
        assert_eq!(lines[..9], want);
    }

    /// The light outlines (a stroke t = 2 px wide on its middle) the frame drew.
    fn outlines(f: &Frame) -> Vec<Rect> {
        (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.stroke.width == 2.0 && r.stroke_kind == egui::StrokeKind::Middle =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn only_the_current_tracks_current_clip_has_the_light_outline() {
        // A one-bar Drums clip from beat 0 (x 50 … 203), and the MiniSynth's (x 203 … 357); the outline is 1.5t (3 px)
        // above and below a clip box (Y_r + 1 … Y_{r+1} − t − 1).
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(0, 4.0);
        let mut rig = Rig::new(song);
        let lit = |x: [f32; 2], row: f32| {
            let y_r = 48.0 + 64.0 * row;
            Rect::from_min_max(
                pos2(x[0], y_r + 1.0 - 3.0),
                pos2(x[1], y_r + 64.0 - 3.0 + 3.0),
            )
        };
        assert_eq!(outlines(&rig.frame(vec![])), []);
        // A tap makes the MiniSynth's clip current, and its track the current track.
        rig.click(CLIP);
        let synth = lit([203.0, 357.0], 2.0);
        assert_eq!(outlines(&rig.frame(vec![])), [synth]);
        // A long press selects the Drums clip with its handles; the current clip stays the only one outlined.
        rig.long_press(pos2(120.0, 144.0));
        let drums = ClipRef {
            channel: 1,
            lane: 0,
            id: 1,
        };
        assert_eq!(rig.screen.clips.selected, Some(drums));
        assert_eq!(outlines(&rig.frame(vec![])), [synth]);
        // The Drums track made current (a tap on its empty space after the one that deselects): its track has no
        // current clip, and the MiniSynth's track is no longer current.
        rig.click(pos2(600.0, 144.0));
        rig.click(pos2(600.0, 144.0));
        assert_eq!(outlines(&rig.frame(vec![])), []);
    }

    /// The miniature bars (t = 2 px tall) on track row `row`, left to right: (x, colour).
    fn bars(f: &Frame, row: f32) -> Vec<(f32, Color32)> {
        let ys = Rangef::new(48.0 + 64.0 * row, 48.0 + 64.0 * (row + 1.0));
        let mut bars: Vec<(f32, Color32)> = (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.height() == 2.0 && ys.contains(r.rect.top()) => {
                    Some((r.rect.left(), r.fill))
                }
                _ => None,
            })
            .collect();
        bars.sort_by(|a, b| a.0.total_cmp(&b.0));
        bars
    }

    /// The MiniSynth's clip shown twice (repeats at x 203 and 357; notes on beats 0 and 2 of each), and a Drums clip
    /// with a hit at beat 0.
    fn song_with_repeats() -> Song {
        let mut song = song_with_a_clip();
        song.channels[2].lanes[0].clips[0].visible_len_beats = 8.0;
        song.channels[1].lanes[0].add_clip(0, 4.0).events = vec![Event::Drum {
            pos: 0,
            len_beats: 0.25,
            pad: 0,
            velocity: 100,
            pan: Event::DEFAULT_PAN,
            value: 0x7FFF,
            repeat: 0,
        }];
        song
    }

    #[test]
    fn a_muted_clip_dims_every_repeat() {
        let mut rig = Rig::new(song_with_repeats());
        let plain = bars(&rig.frame(vec![]), 2.0);
        assert_eq!(
            plain.iter().map(|b| b.0).collect::<Vec<_>>(),
            [203.0, 280.0, 357.0, 434.0]
        );
        let bright = plain[0].1;
        assert!(plain.iter().all(|b| b.1 == bright));
        rig.song.channels[2].lanes[0].clips[0].muted = true;
        let muted = bars(&rig.frame(vec![]), 2.0);
        let dim = muted[0].1;
        assert_ne!(dim, bright);
        assert!(muted.iter().all(|b| b.1 == dim));
    }

    #[test]
    fn preview_dims_every_clip_but_the_one_it_plays() {
        let mut rig = Rig::new(song_with_repeats());
        let f = rig.frame(vec![]);
        let (synth, drums) = (bars(&f, 2.0), bars(&f, 1.0));
        assert_eq!(drums.len(), 1);
        rig.preview = Some(ClipKey {
            channel: 2,
            lane: 0,
            id: 1,
        });
        let f = rig.frame(vec![]);
        assert_eq!(bars(&f, 2.0), synth);
        assert_ne!(bars(&f, 1.0)[0].1, drums[0].1);
    }

    #[test]
    fn a_selected_clip_dims_its_repeats_but_the_one_under_the_selection_while_its_handles_show() {
        let mut rig = Rig::new(song_with_repeats());
        let bright = bars(&rig.frame(vec![]), 2.0)[0].1;
        // A tap on repeat 1 (beat 6 of the clip, x 434) selects the clip there with its handles: repeat 0 dims.
        rig.click(pos2(434.0, 207.5));
        let inks: Vec<Color32> = bars(&rig.frame(vec![]), 2.0).iter().map(|b| b.1).collect();
        let dim = inks[0];
        assert_ne!(dim, bright);
        assert_eq!(inks, [dim, dim, bright, bright]);
        let inks = |rig: &mut Rig, row| -> Vec<Color32> {
            bars(&rig.frame(vec![]), row).iter().map(|b| b.1).collect()
        };
        // A drag of the end handle (at x 510.8; pressed 6 points right of it, past the clip on its repeat 2) four beats
        // on: three repeats, and the handle's touch-down kept the picked one.
        let handle = pos2(510.8 + 6.0, 207.5);
        rig.drag(handle, handle + vec2(4.0 * BEAT_PT, 0.0));
        assert_eq!(rig.clips(2)[0].visible_len_beats, 12.0);
        assert_eq!(inks(&mut rig, 2.0), [dim, dim, bright, bright, dim, dim]);
        // The MiniSynth and the Drums swap places (a track reorder): the selection follows to row 1 with its repeat.
        rig.song.channels.swap(1, 2);
        let swap = |c, l| Some((if c == 1 || c == 2 { 3 - c } else { c }, l));
        rig.screen.clips.rekey(swap);
        rig.selected = 1;
        assert_eq!(inks(&mut rig, 1.0), [dim, dim, bright, bright, dim, dim]);
        // While a touch hides the handles, nothing is dimmed.
        let body = pos2(278.4, 143.5);
        rig.frame(vec![egui::Event::PointerMoved(body)]);
        rig.frame(vec![button(body, true)]);
        assert_eq!(inks(&mut rig, 1.0), [bright; 6]);
    }

    /// The selection's fill boxes (`text` at α 0.10 … 0.20) the frame drew, with their alpha.
    fn selection_fills(f: &Frame) -> Vec<(Rect, u8)> {
        let [tr, tg, tb, _] = theme_colors().text.to_color32().to_srgba_unmultiplied();
        (rects(f).into_iter())
            .filter_map(|(r, fill)| {
                let [cr, cg, cb, a] = fill.to_srgba_unmultiplied();
                // Unmultiplied from about 26/255 of alpha: a few steps off.
                let near = |x: u8, y: u8| x.abs_diff(y) <= 8;
                let text = near(cr, tr) && near(cg, tg) && near(cb, tb);
                (text && (25..=52).contains(&a)).then_some((r, a))
            })
            .collect()
    }

    /// Every stroked rect the frame drew over the track rows.
    fn strokes(f: &Frame) -> Vec<Rect> {
        (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.stroke.width > 0.0
                        && r.rect.left() >= GRID_X
                        && r.rect.top() >= 48.0
                        && r.rect.bottom() <= 380.0 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    /// A press held on empty space at `from`, then dragged to `to` and released: a marquee.
    fn marquee(rig: &mut Rig, [from, to]: [Pos2; 2]) {
        rig.frame(vec![egui::Event::PointerMoved(from)]);
        rig.frame(vec![button(from, true)]);
        rig.frames(vec![vec![]; 4]);
        rig.frame(vec![egui::Event::PointerMoved(to)]);
        rig.frame(vec![button(to, false)]);
    }

    /// From the master's empty row (x 400) to (100, 200): across the Drums clip of [`grouped`] and the MiniSynth's.
    const ACROSS: [Pos2; 2] = [pos2(400.0, 80.0), pos2(100.0, 200.0)];

    /// [`song_with_a_clip`] with a one-bar Drums clip from beat 0 (row 1, x 50 … 203.6) besides the MiniSynth's (row 2,
    /// x 203.6 … 357.2).
    fn grouped() -> Song {
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(0, 4.0);
        song
    }

    #[test]
    fn a_marquee_group_gets_one_pulsing_box_over_its_clips_and_no_outline() {
        // One box from the earliest start to the latest end, the first selected row's top (Y_1 = 112) to the last's
        // bottom (Y_3 = 240); only the marquee itself is drawn (no outline on the clips after).
        let mut rig = Rig::new(grouped());
        marquee(&mut rig, ACROSS);
        assert_eq!(rig.screen.clips.group.len(), 2);
        let f = rig.frame(vec![]);
        let sbox = Rect::from_min_max(pos2(GRID_X, 112.0), pos2(357.2, 240.0));
        let fills: Vec<Rect> = selection_fills(&f).into_iter().map(|f| f.0).collect();
        assert_eq!(fills, [sbox]);
        assert_eq!(strokes(&f), []);
    }

    #[test]
    fn the_selection_box_pulses_text_at_0_15_plus_0_05_cos_2_pi_p_with_p_stepping_0_0125_a_frame() {
        // A long press selects the MiniSynth's clip without making it current: its box (x 203.6 … 357.2, y 176 … 240)
        // and no outline at all. The box shows from the press: the long press drew it 4 frames (p = 0 … 0.0375), so
        // over the next 81 frames the alpha runs the 80-frame cosine from p = 0.05, the clips asking for every frame.
        let mut rig = Rig::new(song_with_a_clip());
        rig.long_press(CLIP);
        let sbox = Rect::from_min_max(pos2(203.6, 176.0), pos2(357.2, 240.0));
        let asks =
            |rig: &Rig| (rig.ctx.repaint_causes().iter()).any(|c| c.file.ends_with("clips.rs"));
        let alphas: Vec<u8> = (0..81)
            .map(|_| {
                let f = rig.frame(vec![]);
                assert_eq!(strokes(&f), []);
                assert!(asks(&rig));
                let fills = selection_fills(&f);
                assert_eq!(fills.len(), 1);
                assert_eq!(fills[0].0, sbox);
                fills[0].1
            })
            .collect();
        let alpha = |p: f32| {
            let a = 0.15 + 0.05 * (std::f32::consts::TAU * p).cos();
            Rgba {
                a,
                ..theme_colors().text
            }
            .to_color32()
            .to_srgba_unmultiplied()[3]
        };
        // The phase that fits best, and it fits to a step of alpha everywhere.
        let off = |j: usize| -> Vec<u8> {
            (alphas.iter().enumerate())
                .map(|(k, &a)| a.abs_diff(alpha((j + k) as f32 * 0.0125)))
                .collect()
        };
        let phase = (0..80).min_by_key(|&j| off(j).iter().map(|&d| u32::from(d)).sum::<u32>());
        assert_eq!(phase, Some(4), "{alphas:?}");
        assert!(off(4).iter().all(|&d| d <= 1), "{alphas:?}");
        assert_eq!(alphas.iter().min().unwrap().abs_diff(26), 0, "0.10");
        assert!(alphas.iter().max().unwrap().abs_diff(51) <= 1, "0.20");
        // Deselected: no box, and nothing asks for frames.
        rig.click(pos2(600.0, CLIP.y));
        let f = rig.frame(vec![]);
        assert_eq!(selection_fills(&f), []);
        rig.frames(vec![vec![]; 10]);
        assert!(!asks(&rig));
    }

    /// The handles' discs over the track rows: (centre to a hundredth of a point, radius) of the `text` ones.
    fn handle_discs(f: &Frame) -> Vec<(Pos2, f32)> {
        let r = |v: f32| (v * 100.0).round() / 100.0;
        (grid_discs(f).into_iter())
            .filter(|c| c.2)
            .map(|c| (pos2(r(c.0.x), r(c.0.y)), c.1))
            .collect()
    }

    /// The `text` discs over the playlist grid (centre, radius, on a `background` α 0.3 rim): the handles have the rim,
    /// the floating row's light bubbles none.
    fn grid_discs(f: &Frame) -> Vec<(Pos2, f32, bool)> {
        let c = theme_colors();
        let (text, rim) = (
            c.text.to_color32(),
            Rgba {
                a: 0.3,
                ..c.background
            }
            .to_color32(),
        );
        let all = circles(f);
        let rimmed = |p: Pos2| all.iter().any(|c| c.0 == p && c.2 == rim);
        (all.iter())
            .filter(|c| {
                c.2 == text && (48.0..380.0).contains(&c.0.y) && c.0.x < 1100.0 && c.1 < 31.0
            })
            .map(|c| (c.0, c.1, rimmed(c.0)))
            .collect()
    }

    /// [`song_with_a_clip`] with an Audio Clip channel (3) whose one-bar clip from beat 0 does not loop: row 3, x 50 …
    /// 203.6, y 240 … 304.
    fn with_audio() -> Song {
        let mut song = song_with_a_clip();
        song.add_channel(ChannelKind::AudioClip);
        song.channels[3].lanes[0].add_clip(0, 4.0);
        song
    }

    #[test]
    fn the_handles_are_half_u_discs_on_the_left_edge_0_65_u_past_the_right_and_under_an_audio_clips_corner()
     {
        let c = theme_colors();
        let rim = Rgba {
            a: 0.3,
            ..c.background
        }
        .to_color32();
        // A notes clip: two handles at mid row (y 208), each a `background` α 0.3 disc of 16 under a `text` one of 15.
        let mut rig = Rig::new(song_with_a_clip());
        rig.long_press(CLIP);
        rig.frames(vec![vec![]; 3]);
        let f = rig.frame(vec![]);
        let (left, right) = (pos2(203.6, 208.0), pos2(378.0, 208.0));
        assert_eq!(handle_discs(&f), [(left, 15.0), (right, 15.0)]);
        let rims: Vec<(Pos2, f32)> = (circles(&f).into_iter())
            .filter(|c| c.2 == rim)
            .map(|c| (c.0, c.1))
            .collect();
        assert_eq!(rims, [(left, 16.0), (right, 16.0)]);
        // An audio clip that does not loop: the third, at (its right edge, Y_4 + 0.8 U).
        let mut rig = Rig::new(with_audio());
        rig.long_press(pos2(120.0, 272.0));
        rig.frames(vec![vec![]; 3]);
        let discs = handle_discs(&rig.frame(vec![]));
        let (left, right, third) = (pos2(GRID_X, 272.0), pos2(224.4, 272.0), pos2(203.6, 329.6));
        assert_eq!(discs, [(left, 15.0), (right, 15.0), (third, 15.0)]);
        // Looping, no third.
        rig.song.channels[3].lanes[0].clips[0].visible_len_beats = 5.0;
        let discs = handle_discs(&rig.frame(vec![]));
        assert_eq!(discs.len(), 2);
        // Nor with two clips selected, even two audio clips that do not loop (the second from beat 5, x 242 … 395.6).
        let mut song = with_audio();
        song.channels[3].lanes[0].add_clip(5 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        marquee(&mut rig, [pos2(414.8, 280.0), pos2(70.0, 300.0)]);
        assert_eq!(rig.screen.clips.group.len(), 2);
        rig.frames(vec![vec![]; 3]);
        let discs = handle_discs(&rig.frame(vec![]));
        assert_eq!(
            discs,
            [(pos2(GRID_X, 272.0), 15.0), (pos2(416.4, 272.0), 15.0)]
        );
    }

    #[test]
    fn a_handles_touch_hides_by_its_rule_and_a_clips_touch_hides_them_all() {
        // The audio clip's handles: left (x 50), right (224.4) at y 272, third (203.6, 329.6).
        let mut rig = Rig::new(with_audio());
        rig.long_press(pos2(120.0, 272.0));
        rig.frames(vec![vec![]; 3]);
        let shown = |rig: &mut Rig, at: Pos2| -> Vec<f32> {
            rig.frame(vec![egui::Event::PointerMoved(at)]);
            rig.frame(vec![button(at, true)]);
            let xs = handle_discs(&rig.frame(vec![]))
                .iter()
                .map(|d| d.0.x)
                .collect();
            // Held past 0.3 s: a long press, which keeps the selection (a tap would toggle it).
            rig.frames(vec![vec![]; 3]);
            rig.frame(vec![button(at, false)]);
            rig.frames(vec![vec![]; 3]);
            xs
        };
        // The right one hides the third; the third hides the other two; the left one hides none.
        assert_eq!(shown(&mut rig, pos2(224.4, 272.0)), [GRID_X, 224.4]);
        assert_eq!(shown(&mut rig, pos2(203.6, 329.6)), [203.6]);
        assert_eq!(
            shown(&mut rig, pos2(GRID_X + 4.0, 272.0)),
            [GRID_X, 224.4, 203.6]
        );
        // A touch on the clip hides them all.
        assert_eq!(shown(&mut rig, pos2(120.0, 272.0)), Vec::<f32>::new());
    }

    #[test]
    fn the_handles_pop_in_grow_while_held_and_wobble_once_let_go() {
        // The other editor handles' animations, e.g. the audio editor's stretch handle. The pop-in is the editors'
        // 1.0515·sin(1.885·(1 − a)), a ← 0.6·a a 60 Hz frame (the rig's frames are 0.1 s: 6 of them); held, a disc
        // grows by up to 10 px over 0.1 s (5 a side); let go, it wobbles from there, 10·cos(2πs/0.12)·(1 − s/0.3) px
        // for 0.3 s (the knob law).
        let a = |frames: u32| crate::ui::step_seq::pop_in(0.6, f64::from(frames) * 0.1);
        let right = |f: &Frame| {
            handle_discs(f)
                .into_iter()
                .find(|d| d.0.x == 378.0)
                .map(|d| d.1)
        };
        let near = |r: f32, want: f32| (r - want).abs() < 1e-3;
        let mut rig = Rig::new(song_with_a_clip());
        rig.long_press(CLIP);
        let pop: Vec<f32> = (0..3).map(|_| right(&rig.frame(vec![])).unwrap()).collect();
        let scale = |k| crate::ui::automation::pop_scale(a(k));
        assert_eq!(pop, [15.0 * scale(0), 15.0 * scale(1), 15.0]);
        // Held (a long press, which keeps the selection): +10 px, 20 of radius, from 0.1 s on.
        let at = pos2(378.0, 208.0);
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        rig.frame(vec![button(at, true)]);
        for _ in 0..4 {
            let held = right(&rig.frame(vec![])).unwrap();
            assert!(near(held, 20.0), "{held}");
        }
        rig.frame(vec![button(at, false)]);
        let wobble = |s: f64| {
            15.0 + (10.0 * (std::f64::consts::TAU * s / 0.12).cos() * (1.0 - s / 0.3)) as f32 / 2.0
        };
        for s in [0.1, 0.2, 0.3] {
            let r = right(&rig.frame(vec![])).unwrap();
            assert!(near(r, wobble(s)), "{s}: {r} vs {}", wobble(s));
        }
        assert_eq!(right(&rig.frame(vec![])), Some(15.0));
    }

    #[test]
    fn a_live_marquee_draws_no_selection_and_its_release_shows_it_popping_in_from_p_0() {
        // The handles come on release after a drag. A clip selected for a while, then a marquee from the master's empty
        // row across the Drums and MiniSynth clips: while it is live its group shows no box and no handles; released,
        // both show, the handles' pop-in from its start (radius 0) and the fill from p = 0 (α 0.20).
        let mut rig = Rig::new(grouped());
        rig.long_press(CLIP);
        rig.frames(vec![vec![]; 10]);
        let [from, to] = ACROSS;
        rig.frame(vec![egui::Event::PointerMoved(from)]);
        rig.frame(vec![button(from, true)]);
        rig.frames(vec![vec![]; 4]);
        rig.frame(vec![egui::Event::PointerMoved(to)]);
        let f = rig.frame(vec![]);
        assert_eq!(rig.screen.clips.group.len(), 2);
        // The only box is the marquee's own (`text` α 0.2).
        let marquee = Rect::from_two_pos(from, to);
        assert_eq!(selection_fills(&f), [(marquee, 51)]);
        assert_eq!(handle_discs(&f), []);
        rig.frame(vec![button(to, false)]);
        let f = rig.frame(vec![]);
        let sbox = Rect::from_min_max(pos2(GRID_X, 112.0), pos2(357.2, 240.0));
        let alpha = Rgba {
            a: 0.2,
            ..theme_colors().text
        };
        assert_eq!(
            selection_fills(&f),
            [(sbox, alpha.to_color32().to_srgba_unmultiplied()[3])]
        );
        assert_eq!(
            handle_discs(&f),
            [(pos2(GRID_X, 176.0), 0.0), (pos2(378.0, 176.0), 0.0)]
        );
    }

    #[test]
    fn a_touch_in_the_groups_frame_hides_its_handles() {
        // The group's box, on its empty part (row 1 right of the Drums clip): the touch acts on the group whole.
        let mut rig = Rig::new(grouped());
        marquee(&mut rig, ACROSS);
        rig.frames(vec![vec![]; 3]);
        assert_eq!(handle_discs(&rig.frame(vec![])).len(), 2);
        let p = pos2(300.0, 144.0);
        rig.frame(vec![egui::Event::PointerMoved(p)]);
        rig.frame(vec![button(p, true)]);
        let f = rig.frame(vec![]);
        assert_eq!(handle_discs(&f), []);
        assert_eq!(selection_fills(&f).len(), 1);
    }

    #[test]
    fn a_handles_tap_only_deselects_for_a_clip_and_for_a_group() {
        // An empty-space tap, wherever the handle: the selection goes and no clip becomes current.
        let mut rig = Rig::new(grouped());
        rig.click(CLIP);
        rig.frames(vec![vec![]; 3]);
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        // The current clip's right handle (x 378, y 208).
        rig.click(pos2(378.0, 208.0));
        assert_eq!(rig.screen.clips.selected, None);
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        // Another clip's (the Drums clip long pressed, not current: its right handle at x 224.4, y 144).
        rig.long_press(pos2(120.0, 144.0));
        rig.frames(vec![vec![]; 3]);
        rig.click(pos2(224.4, 144.0));
        assert_eq!(rig.screen.clips.selected, None);
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        // The group's right handle (x 378, y 176).
        marquee(&mut rig, ACROSS);
        rig.frames(vec![vec![]; 3]);
        assert_eq!(rig.screen.clips.group.len(), 2);
        rig.click(pos2(378.0, 176.0));
        assert_eq!(rig.screen.clips.group, []);
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
    }

    #[test]
    fn the_third_handles_drag_stretches_a_decodable_sample_on_release() {
        // 2 s of silence at 44.1 kHz in the user folder, at 120 BPM: 4 beats, the audio clip's whole loop. Its third
        // handle (x 203.6, y 329.6) dragged two beats on: the end law (6 beats), then on release the audio editor's
        // stretch for 1.5 × the sample's 4 beats, the STR1 ratio 1.5 and SYNC on.
        let mut song = with_audio();
        song.tempo_bpm = 120.0;
        let path = crate::flm::chunk::ContentPath::Shorthand {
            root: crate::flm::chunk::ContentRoot::User,
            relative: "My Samples/silence.wav".into(),
        };
        let mut slot = crate::song::SamplerSlot::default();
        slot.file.path = Some(path);
        song.channels[3].lanes[0].clips[0].file.sample = Some(Box::new(slot));
        let wav = crate::sample_cache::tests::wav(&[0; 88_200]);
        USER_FILES.with_borrow_mut(|f| *f = vec![("My Samples/silence.wav", wav)]);
        let mut rig = Rig::new(song);
        rig.click(pos2(120.0, 272.0));
        rig.frames(vec![vec![]; 3]);
        let third = pos2(203.6, 329.6);
        let edits = rig.drag(third, third + vec2(2.0 * BEAT_PT, 0.0));
        USER_FILES.with_borrow_mut(Vec::clear);
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
        let clip = &rig.song.channels[3].lanes[0].clips[0];
        let slot = clip.file.sample.as_deref().unwrap();
        assert_eq!(clip.visible_len_beats, 6.0);
        assert_eq!((slot.file.stretch_mode, slot.file.stretch_ratio), (1, 1.5));
    }

    #[test]
    fn a_groups_handles_resize_each_of_its_clips_by_the_handle_law() {
        // The group's right handle (x 378, y 176) drags every clip's end by the same snapped beat; the left one
        // (x 50) every start, each at most its end less half a step.
        let mut rig = Rig::new(grouped());
        marquee(&mut rig, ACROSS);
        rig.frame(vec![]);
        let right = pos2(378.0, 176.0);
        rig.drag(right, right + vec2(BEAT_PT, 0.0));
        let len = |rig: &Rig, c: usize| {
            rig.clips(c)
                .iter()
                .map(|c| c.visible_len_beats)
                .collect::<Vec<_>>()
        };
        assert_eq!((len(&rig, 1), len(&rig, 2)), (vec![5.0], vec![5.0]));
        rig.frame(vec![]);
        let left = pos2(GRID_X + 4.0, 176.0);
        rig.drag(left, left + vec2(9.0 * BEAT_PT, 0.0));
        let starts = |rig: &Rig, c: usize| {
            rig.clips(c)
                .iter()
                .map(|c| c.start_tick)
                .collect::<Vec<_>>()
        };
        // Ends at beats 5 and 9: starts 16 ticks (half a step at d = 4) before them.
        assert_eq!(
            (starts(&rig, 1), starts(&rig, 2)),
            (vec![640 - 16], vec![1152 - 16])
        );
    }

    #[test]
    fn a_clip_being_recorded_shows_only_as_its_live_bar() {
        // The live take bar: the take's clip (MiniSynth row, y 176 ..= 239, from beat 4) shows as the bar from its
        // start to the playhead (beat 5), not as itself: nothing of it past the bar.
        let mut rig = Rig::new(song_with_a_clip());
        let fills_past = |rig: &mut Rig| {
            let f = rig.frame(vec![]);
            let past = pos2(48.0 + 6.5 * BEAT_PT, 207.5);
            (f.shapes.iter())
                .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect.contains(past)))
                .count()
        };
        let drawn = fills_past(&mut rig);
        let id = rig.song.channels[2].lanes[0].clips[0].id;
        rig.screen.takes = vec![LiveTake {
            channel: 2,
            lane: 0,
            start: 512.0,
            end: 640.0,
            clip: Some(id),
        }];
        assert!(
            fills_past(&mut rig) < drawn,
            "the clip is not drawn past the bar"
        );
    }

    #[test]
    fn the_armed_dot_is_a_tenth_of_u_three_tenths_in_from_the_headers_corner() {
        for (ppp, scale) in [(1.0, DEFAULT_UI_SCALE_INDEX), (2.0, 7)] {
            let m = Metrics::from_pixels_per_point(ppp, scale);
            let header = Rect::from_min_size(pos2(0.0, 100.0), vec2(m.u(1.5), m.u(2.0)));
            let (center, r) = armed_dot(header, &m);
            assert_eq!(center, pos2(m.u(0.3), 100.0 + m.u(0.3)), "ppp {ppp}");
            assert_eq!(r, m.u(0.1), "ppp {ppp}");
        }
    }

    #[test]
    fn a_ruler_range_is_the_play_loop_and_its_plus_menu_edits_the_song_in_one_final_edit() {
        let mut rig = Rig::new(song_with_a_clip());
        // Beats 2 to 6, across the clip's start (it plays 60 at beat 4 and 67 at beat 6).
        rig.select(2.0, 6.0);
        assert_eq!(rig.song.play_loop, Some((256, 768)));
        rig.click_id(Id::new("playlist_region").with("plus"));
        // `Delete space`: the clip's part in the range goes, its rest moves 4 beats earlier; the loop goes too.
        assert_eq!(rig.region_item(6), Some(SongEdit::Final));
        assert_eq!(plays(&rig.song, 2), [(256, 67)]);
        assert_eq!(rig.song.play_loop, None);
        // `Duplicate` keeps it.
        rig.region(2.0, 4.0);
        assert_eq!(rig.region_item(2), Some(SongEdit::Final));
        assert_eq!(rig.song.play_loop, Some((256, 512)));
        assert_eq!(plays(&rig.song, 2), [(256, 67), (512, 67)]);
    }

    #[test]
    fn the_playlists_playhead_items_need_the_playhead_outside_the_range() {
        // No output: the playhead is at tick 0.
        let mut rig = Rig::new(song_with_a_clip());
        // From beat 2 back to beat 0, under the playhead knob (its square reaches 0.84 beat): beats 0 to 2.
        rig.region(2.0, 0.0);
        assert_eq!(rig.song.play_loop, Some((0, 256)));
        assert_eq!(rig.region_item(3), None);
        assert_eq!(rig.clips(2).len(), 1);
        rig.frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        // The clip's bar: `Copy to playhead` inserts a copy of it at the song start, pushing the rest.
        rig.region(4.0, 8.0);
        assert_eq!(rig.region_item(3), Some(SongEdit::Final));
        assert_eq!(
            plays(&rig.song, 2),
            [(0, 60), (256, 67), (1024, 60), (1280, 67)]
        );
    }

    #[test]
    fn a_tap_on_the_ruler_asks_for_the_playhead_on_the_nearest_grid_line() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.frame(vec![egui::Event::PointerMoved(ruler(3.4))]);
        rig.frame(vec![button(ruler(3.4), true)]);
        // The beat grid at the default zoom.
        assert_eq!(
            rig.frame(vec![button(ruler(3.4), false)]).playhead,
            Some(384)
        );
        assert_eq!(rig.song.play_loop, None);
    }

    #[test]
    fn a_tap_makes_a_clip_current_and_selected_and_a_second_tap_on_it_deselects_it() {
        let mut rig = Rig::new(song_with_a_clip());
        assert_eq!(rig.click(CLIP), (None, None));
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        assert_eq!(rig.selected, 2, "its channel takes the keyboard");
        // Current and selected: the next tap (released 0.4 s later, no double tap) deselects it; it stays current.
        rig.frame(vec![]);
        assert_eq!(rig.click(CLIP), (None, None));
        assert_eq!(rig.screen.clips.selected, None);
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        // And the next selects it again.
        rig.frame(vec![]);
        rig.click(CLIP);
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
    }

    #[test]
    fn a_tap_beside_a_clip_deselects_then_shows_clip_plus_on_the_current_track_whose_press_adds_a_bar()
     {
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        // Past the clip's end and its right handle (to x 394), in beat 9.2 of the same (current) track: first it only
        // deselects.
        let beside = pos2(GRID_X + 9.2 * BEAT_PT, CLIP.y);
        assert_eq!(rig.click(beside), (None, None));
        assert_eq!(rig.screen.clips.selected, None);
        assert!(!rig.plus_shown());
        // Then `+Clip`: a 1.5 U (48-point) square centred on the tap's x and the row's centre line.
        assert_eq!(rig.click(beside), (None, None));
        let plus = rig.rect_of(Id::new("clip_plus")).expect("+Clip shows");
        assert_eq!(
            plus,
            Rect::from_center_size(pos2(beside.x, CLIP.y), vec2(48.0, 48.0))
        );
        assert_eq!(rig.clips(2).len(), 1);
        // Its press adds a bar at the snap line nearest its centre (beat 9: a beat at this zoom), current and selected.
        assert_eq!(rig.plus().0, Some(SongEdit::Final));
        let added = &rig.clips(2)[1];
        assert_eq!(
            (added.id, added.start_tick, added.loop_len_beats),
            (2, 1152, 4.0)
        );
        let new = ClipRef {
            id: 2,
            ..SYNTH_CLIP
        };
        assert_eq!(rig.screen.clips.selected, Some(new));
        assert_eq!(rig.current_clip(), Some(new));
        assert!(!rig.plus_shown());
        // Any other press hides `+Clip`.
        let further = pos2(GRID_X + 14.3 * BEAT_PT, CLIP.y);
        rig.click(further);
        rig.click(further);
        assert!(rig.plus_shown());
        rig.click(pos2(600.0, 80.0));
        assert!(!rig.plus_shown());
    }

    #[test]
    fn a_tap_on_another_tracks_empty_space_makes_it_current_and_only_note_and_drum_tracks_take_clips()
     {
        let mut rig = Rig::new(song_with_a_clip());
        rig.selected = 2;
        // The drums' track is not current: the first tap makes it so (its channel takes the keyboard).
        let drums = pos2(GRID_X + 0.3 * BEAT_PT, 144.0);
        assert_eq!(rig.click(drums), (None, None));
        assert_eq!(rig.selected, 1);
        assert!(!rig.plus_shown());
        rig.click(drums);
        assert_eq!(rig.plus().0, Some(SongEdit::Final));
        assert_eq!(rig.clips(1)[0].start_tick, 0);
        // The master's automation track never shows `+Clip` (and never takes the keyboard).
        let master = pos2(600.0, 80.0);
        for _ in 0..3 {
            assert_eq!(rig.click(master), (None, None));
            assert!(!rig.plus_shown());
        }
        assert_eq!(rig.clips(0).len(), 1, "only the rig's own clip at beat 60");
        assert_eq!(rig.selected, 1);
    }

    #[test]
    fn a_double_tap_on_a_clip_opens_its_editor_and_the_window_is_0_3_s_release_to_release() {
        let mut rig = Rig::new(song_with_a_clip());
        let edit = Nav::EditClip(ClipKey {
            channel: 2,
            lane: 0,
            id: 1,
        });
        // Select, then within 0.3 s open; the second tap leaves the selection as it was.
        assert_eq!(rig.double_tap(CLIP), (None, Some(edit)));
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        // Released 0.3 s apart: no double tap (the second tap deselects).
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.click(CLIP), (None, None));
        assert_eq!(rig.screen.clips.selected, None);
        // A quick tap on a clip that is not current never opens: it makes it current.
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(0, 4.0);
        let mut rig = Rig::new(song);
        let drums = pos2(GRID_X + BEAT_PT, 144.0);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, true)],
            vec![button(CLIP, false)],
            vec![egui::Event::PointerMoved(drums)],
            vec![button(drums, true)],
        ]);
        assert_eq!(rig.frame(vec![button(drums, false)]).nav, None);
        let drum_clip = ClipRef {
            channel: 1,
            lane: 0,
            id: 1,
        };
        assert_eq!(rig.current_clip(), Some(drum_clip));
    }

    #[test]
    fn a_long_press_on_a_clip_shows_its_handles_and_row_without_making_it_current() {
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(0, 4.0);
        let mut rig = Rig::new(song);
        rig.click(CLIP);
        let drums = pos2(GRID_X + BEAT_PT, 144.0);
        rig.long_press(drums);
        let drum_clip = ClipRef {
            channel: 1,
            lane: 0,
            id: 1,
        };
        assert_eq!(rig.screen.clips.selected, Some(drum_clip));
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        assert_eq!(rig.selected, 2);
        assert!(rig.rect_of(Id::new(("clip_button", 0_usize))).is_some());
        // A long press on the selected clip keeps it selected.
        rig.long_press(drums);
        assert_eq!(rig.screen.clips.selected, Some(drum_clip));
    }

    /// The filled rects of `f` centred on `at`, as their sizes.
    fn squares(f: &Frame, at: Pos2) -> Vec<f32> {
        (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if (r.rect.center() - at).length() < 0.5 => {
                    Some(r.rect.width())
                }
                _ => None,
            })
            .collect()
    }

    /// The song with a drum clip under the MiniSynth's (both beats 4 to 8, rows 1 and 2).
    fn two_rows() -> Song {
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(512, 4.0);
        song
    }

    const DRUM_CLIP: ClipRef = ClipRef {
        channel: 1,
        lane: 0,
        id: 1,
    };

    #[test]
    fn the_shared_long_press_time_and_calm_zone_decide_a_touch_on_a_clip() {
        // `gesture::LONG_PRESS_S` and `CALM_U` (0.3 s; 0.5 U, 16 points at U = 32) on a clip that is not current:
        // released at 0.28 s, or moved 0.45 U, the press is a tap (it makes the clip current); at 0.32 s it is a long
        // press (selected, not current); moved 0.55 U before its 300 ms it is cancelled (nothing).
        let after = |secs: f64, by: f32| {
            let mut rig = Rig::new(song_with_a_clip());
            rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
            rig.frame(vec![button(CLIP, true)]);
            let mut to = CLIP;
            // Each frame is 0.1 s on, the release's too.
            let mut held = 0.1;
            if by != 0.0 {
                to = CLIP + vec2(by, 0.0);
                rig.frame(vec![egui::Event::PointerMoved(to)]);
                held += 0.1;
            }
            rig.time += secs - held;
            rig.frame(vec![button(to, false)]);
            (rig.current_clip(), rig.screen.clips.selected)
        };
        let synth = Some(SYNTH_CLIP);
        assert_eq!(after(0.28, 0.0), (synth, synth));
        assert_eq!(after(0.32, 0.0), (None, synth));
        assert_eq!(after(0.2, 0.45 * 32.0), (synth, synth));
        assert_eq!(after(0.2, 0.55 * 32.0), (None, None));
    }

    #[test]
    fn a_long_press_on_empty_space_grows_the_marquee_and_its_drag_selects_the_clips_it_touches_live()
     {
        let mut rig = Rig::new(two_rows());
        let from = pos2(GRID_X + BEAT_PT, 144.0);
        rig.frame(vec![egui::Event::PointerMoved(from)]);
        rig.frame(vec![button(from, true)]);
        rig.frame(vec![]);
        // 0.2 s in: 3 U·(4.4333·0.2 − 0.33) (U = 32).
        let f = rig.frame(vec![]);
        let side = 96.0 * (0.2 * 4.4333 - 0.33);
        assert!(
            squares(&f, from).iter().any(|w| (w - side).abs() < 0.5),
            "{:?}",
            squares(&f, from)
        );
        rig.frame(vec![]);
        let f = rig.frame(vec![]);
        assert!(!squares(&f, from).iter().any(|w| (w - 96.0).abs() < 0.5));
        assert_eq!(rig.screen.clips.selected, None);
        // Dragged into the MiniSynth's clip: the rectangle touches both clips, the selection already.
        let to = CLIP;
        rig.frame(vec![egui::Event::PointerMoved(to)]);
        assert_eq!(rig.screen.clips.selected, None);
        assert_eq!(rig.screen.clips.group, [DRUM_CLIP, SYNTH_CLIP]);
        rig.frame(vec![button(to, false)]);
        assert!(rig.rect_of(Id::new(("clip_button", 0_usize))).is_some());
        // The group's Delete takes both, as one edit.
        assert_eq!(
            rig.click_id(Id::new(("clip_button", 1_usize))).0,
            Some(SongEdit::Final)
        );
        assert!(rig.clips(1).is_empty() && rig.clips(2).is_empty());
    }

    #[test]
    fn the_group_moves_as_one_and_a_tap_on_one_of_its_clips_makes_it_the_selection() {
        let mut rig = Rig::new(two_rows());
        let from = pos2(GRID_X + BEAT_PT, 144.0);
        let to = CLIP;
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![],
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]);
        assert_eq!(rig.screen.clips.group, [DRUM_CLIP, SYNTH_CLIP]);
        // A drag in its frame moves both a beat on at once.
        let edits = rig.drag(CLIP, CLIP + vec2(BEAT_PT, 0.0));
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
        assert_eq!(
            (rig.clips(1)[0].start_tick, rig.clips(2)[0].start_tick),
            (640, 640)
        );
        // A tap on one: the group goes, the clip is current and selected.
        rig.frame(vec![]);
        rig.click(CLIP + vec2(BEAT_PT, 0.0));
        assert!(rig.screen.clips.group.is_empty());
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
    }

    #[test]
    fn a_shown_rows_circles_pop_in() {
        // From nothing on the frame the row shows.
        let mut rig = Rig::new(song_with_a_clip());
        // The light bubbles: `text` discs with no handle's rim.
        let radii = |f: &Frame| -> Vec<f32> {
            (grid_discs(f).into_iter())
                .filter(|c| !c.2)
                .map(|c| c.1)
                .collect()
        };
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        rig.frame(vec![button(CLIP, true)]);
        let shown = rig.frame(vec![button(CLIP, false)]);
        let first = radii(&shown);
        assert_eq!(first.len(), 5, "{first:?}");
        assert!(first.iter().all(|&r| r < 1.0), "{first:?}");
        // 0.1 s on (6 frames at 60 Hz): past full size; then radius 22.4 points (diameter 1.4 U).
        let next = radii(&rig.frame(vec![]));
        assert!(next.iter().all(|&r| r > 22.4), "{next:?}");
        let settled = radii(&rig.frame(vec![]));
        assert!(
            settled.iter().all(|&r| (r - 22.4).abs() < 1e-3),
            "{settled:?}"
        );
    }

    #[test]
    fn a_clip_that_is_not_current_drags_only_after_300_ms_and_an_earlier_move_cancels_the_touch() {
        let mut rig = Rig::new(song_with_a_clip());
        // Moved 0.1 s after the touch-down: cancelled, the selection hidden, nothing moved.
        let edits = rig.drag(CLIP, CLIP + vec2(2.0 * BEAT_PT, 0.0));
        assert!(edits.iter().all(Option::is_none), "{edits:?}");
        assert_eq!(rig.clips(2)[0].start_tick, 512);
        assert_eq!(rig.screen.clips.selected, None);
        assert_eq!(rig.current_clip(), None);
        // Held 0.4 s first: it drags (and stays not current).
        let to = CLIP + vec2(2.0 * BEAT_PT, 0.0);
        let (edit, _) = rig.frames(vec![
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, true)],
            vec![],
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]);
        assert_eq!(edit, Some(SongEdit::Final));
        assert_eq!(rig.clips(2)[0].start_tick, 768);
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        assert_eq!(rig.current_clip(), None);
    }

    #[test]
    fn dragging_a_clip_moves_it_on_the_beat_grid_and_its_release_is_the_final_edit() {
        let mut rig = Rig::new(song_with_a_clip());
        // Current, it drags at once.
        rig.current();
        // Up and right by 1.4 beats: it stays in its row, one beat later.
        let edits = rig.drag(CLIP, CLIP + vec2(1.4 * BEAT_PT, -30.0));
        assert_eq!(rig.clips(2)[0].start_tick, 640);
        assert!(edits.contains(&Some(SongEdit::Ongoing)), "{edits:?}");
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 2,
                lane: 0,
                id: 1
            })
        );
        // The drag did not pan the playlist.
        assert_eq!((rig.screen.scroll_x, rig.screen.scroll_y), (0.0, 0.0));
    }

    #[test]
    fn a_notes_clip_drags_onto_another_channels_notes_track_but_never_onto_a_drum_or_audio_row() {
        // Clips drag between tracks of any channel by lane kind. Rows: the master, Drums (1), the MiniSynth (2) with
        // its clip, an audio channel (3) and a second MiniSynth (4).
        let mut song = song_with_a_clip();
        song.add_channel(ChannelKind::AudioClip);
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        let mut rig = Rig::new(song);
        rig.current();
        let row = |i: f32| pos2(CLIP.x, 48.0 + 64.0 * i + 32.0);
        // Over the drum row, then the audio row (as near the second synth's row as its own): it stays on its own.
        rig.drag(CLIP, row(1.0));
        rig.drag(CLIP, row(3.0));
        assert_eq!(rig.clips(2).len(), 1);
        // Over the second synth's row: it moves there, still the current and selected clip.
        let edits = rig.drag(CLIP, row(4.0));
        assert!(rig.clips(2).is_empty());
        assert_eq!(rig.clips(4).len(), 1);
        let moved = ClipRef {
            channel: 4,
            lane: 0,
            id: 1,
        };
        assert_eq!(rig.current_clip(), Some(moved));
        assert_eq!(rig.screen.clips.selected, Some(moved));
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
    }

    #[test]
    fn a_tap_on_a_clip_gives_its_channel_the_keyboard_but_the_master_never_takes_it() {
        let mut song = song_with_a_clip();
        // A clip on the master's automation lane, from a file (the rig's song has one at beat 60 already).
        song.channels[0].lanes[0].add_clip(512, 4.0);
        let mut rig = Rig::new(song);
        assert_eq!(rig.selected, 1);
        rig.click(CLIP);
        assert_eq!(rig.selected, 2);
        rig.click(pos2(CLIP.x, 80.0));
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 0,
                lane: 0,
                id: 2
            })
        );
        assert_eq!(rig.selected, 2);
    }

    #[test]
    fn a_drag_ends_with_exactly_one_final_edit_even_when_it_comes_back_to_its_start() {
        let finals = |edits: &[Option<SongEdit>]| {
            edits
                .iter()
                .filter(|&&e| e == Some(SongEdit::Final))
                .count()
        };
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        let away = CLIP + vec2(2.0 * BEAT_PT, 0.0);
        let edits: Vec<_> = [
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, true)],
            vec![egui::Event::PointerMoved(away)],
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, false)],
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert_eq!(rig.clips(2)[0].start_tick, 512);
        assert_eq!(finals(&edits), 1, "{edits:?}");
        let edits = rig.drag(CLIP, away);
        assert_eq!(rig.clips(2)[0].start_tick, 768);
        assert_eq!(finals(&edits), 1, "{edits:?}");
    }

    #[test]
    fn a_pinch_that_starts_on_a_clip_leaves_the_clip_where_it_was() {
        use egui::TouchPhase::{End, Move, Start};
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        // The first finger drives the pointer, as an integration does.
        let first = |phase, pos| {
            let pointer = match phase {
                Start => vec![button(pos, true)],
                Move => vec![egui::Event::PointerMoved(pos)],
                _ => vec![button(pos, false), egui::Event::PointerGone],
            };
            [vec![touch(1, phase, pos)], pointer].concat()
        };
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        // Its visible length is the loop's (stored as 0).
        let before = rig.clips(2)[0].clone();
        let (away, other) = (CLIP + vec2(2.0 * BEAT_PT, 0.0), CLIP + vec2(300.0, 0.0));
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        let edits: Vec<_> = [
            first(Start, CLIP),
            first(Move, away),
            vec![touch(2, Start, other)],
            first(Move, away - vec2(BEAT_PT, 0.0)),
            vec![touch(2, Move, other + vec2(BEAT_PT, 0.0))],
            // The second finger lifts first; the first one moves on.
            vec![touch(2, End, other + vec2(BEAT_PT, 0.0))],
            first(Move, away + vec2(BEAT_PT, 0.0)),
            first(End, away + vec2(BEAT_PT, 0.0)),
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert_eq!(rig.clips(2)[0], before);
        assert_ne!(rig.screen.zoom, 0.0, "no pinch");
        // The drag's ongoing edits, then one final edit when the second finger lands, and nothing after it.
        assert_eq!(edits[1], Some(SongEdit::Ongoing), "{edits:?}");
        assert_eq!(edits[2], Some(SongEdit::Final), "{edits:?}");
        assert!(edits[3..].iter().all(Option::is_none), "{edits:?}");

        // The second finger lands before the first one moves: the drag that starts then writes nothing.
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        let edits: Vec<_> = [
            first(Start, CLIP),
            vec![touch(2, Start, other)],
            first(Move, away),
            vec![touch(2, End, other)],
            first(End, away),
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert_eq!(rig.clips(2)[0], before);
        assert!(edits.iter().all(Option::is_none), "{edits:?}");
    }

    /// Touch `id`'s `phase` at `pos`; the first finger (1) also drives the pointer, as an integration does.
    fn finger(id: u64, phase: egui::TouchPhase, pos: Pos2) -> Vec<egui::Event> {
        use egui::TouchPhase::{Move, Start};
        let touch = egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        let pointer = match (id, phase) {
            (1, Start) => vec![button(pos, true)],
            (1, Move) => vec![egui::Event::PointerMoved(pos)],
            (1, _) => vec![button(pos, false), egui::Event::PointerGone],
            _ => vec![],
        };
        [vec![touch], pointer].concat()
    }

    #[test]
    fn a_pinch_on_a_held_clip_that_is_not_current_puts_the_selection_back() {
        use egui::TouchPhase::{End, Start};
        let mut rig = Rig::new(two_rows());
        // The drum clip selected; the second finger lands while the first holds the synth clip.
        rig.long_press(pos2(CLIP.x, 144.0));
        assert_eq!(rig.screen.clips.selected, Some(DRUM_CLIP));
        let other = CLIP + vec2(300.0, 0.0);
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        rig.frame(finger(1, Start, CLIP));
        assert_eq!(
            rig.screen.clips.selected,
            Some(SYNTH_CLIP),
            "the touch-down's"
        );
        let (edit, _) = rig.frames(vec![
            finger(2, Start, other),
            vec![],
            vec![],
            vec![],
            finger(2, End, other),
            finger(1, End, CLIP),
        ]);
        assert_eq!(edit, None);
        assert_eq!(rig.screen.clips.selected, Some(DRUM_CLIP));
        assert_eq!(rig.clips(2)[0].start_tick, 512);
    }

    #[test]
    fn a_pinch_during_a_group_drag_puts_every_clip_back_in_one_final_edit() {
        use egui::TouchPhase::{End, Move, Start};
        let mut rig = Rig::new(two_rows());
        let from = pos2(GRID_X + BEAT_PT, 144.0);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![],
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, false)],
        ]);
        assert_eq!(rig.screen.clips.group, [DRUM_CLIP, SYNTH_CLIP]);
        let starts = |rig: &Rig| (rig.clips(1)[0].start_tick, rig.clips(2)[0].start_tick);
        let (away, other) = (CLIP + vec2(2.0 * BEAT_PT, 0.0), CLIP + vec2(300.0, 0.0));
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        rig.frame(finger(1, Start, CLIP));
        assert_eq!(
            rig.frame(finger(1, Move, away)).edit,
            Some(SongEdit::Ongoing)
        );
        assert_eq!(starts(&rig), (768, 768), "both moved");
        assert_eq!(
            rig.frame(finger(2, Start, other)).edit,
            Some(SongEdit::Final)
        );
        assert_eq!(starts(&rig), (512, 512), "both back");
        let (edit, _) = rig.frames(vec![
            finger(1, Move, away + vec2(BEAT_PT, 0.0)),
            finger(2, End, other),
            finger(1, End, away),
        ]);
        assert_eq!(edit, None);
        assert_eq!(starts(&rig), (512, 512));
    }

    #[test]
    fn a_quick_pinch_on_an_empty_lane_adds_no_clip() {
        use egui::TouchPhase::{End, Move, Start};
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        // A drum clip at beat 60, so that the time has room to scroll (0 … twice the song's length).
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(60 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        // Beat 8 of the MiniSynth lane; the first finger stays put while the second one moves.
        let (at, other) = (pos2(GRID_X + 8.6 * BEAT_PT, CLIP.y), pos2(800.0, CLIP.y));
        let edits: Vec<_> = [
            vec![egui::Event::PointerMoved(at)],
            vec![touch(1, Start, at), button(at, true)],
            vec![touch(2, Start, other)],
            vec![touch(2, Move, other + vec2(BEAT_PT, 0.0))],
            vec![touch(2, End, other + vec2(BEAT_PT, 0.0))],
            vec![
                touch(1, End, at),
                button(at, false),
                egui::Event::PointerGone,
            ],
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert!(edits.iter().all(Option::is_none), "{edits:?}");
        assert_eq!(rig.clips(2).len(), 1);
        assert_eq!(rig.selected, 1, "the pinch made no track current");
        // Taps after it still do: the track becomes current, then `+Clip` adds a clip.
        rig.click(at);
        assert_eq!(rig.selected, 2);
        rig.click(at);
        assert_eq!(rig.plus().0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2).len(), 2);
    }

    #[test]
    fn a_focus_loss_ends_a_clip_drag_and_a_stale_press_drags_nothing() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        let step = |k: f32| CLIP + vec2(k * BEAT_PT, 0.0);
        let edits: Vec<_> = [
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, true)],
            vec![egui::Event::PointerMoved(step(1.0))],
            vec![egui::Event::WindowFocused(false)],
            // egui keeps the button down: back in the window, the pointer only hovers.
            vec![
                egui::Event::WindowFocused(true),
                egui::Event::PointerMoved(step(3.0)),
            ],
            vec![egui::Event::PointerMoved(step(4.0))],
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert_eq!(rig.clips(2)[0].start_tick, 640);
        assert_eq!(
            edits[2..],
            [Some(SongEdit::Ongoing), Some(SongEdit::Final), None, None]
        );
        // A press gone stale before it moved: the hover that egui takes for its drag moves nothing either.
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        let edits: Vec<_> = [
            vec![egui::Event::PointerMoved(CLIP)],
            vec![button(CLIP, true)],
            vec![egui::Event::WindowFocused(false)],
            vec![egui::Event::PointerMoved(step(1.0))],
            vec![egui::Event::PointerMoved(step(2.0))],
        ]
        .into_iter()
        .map(|events| rig.frame(events).edit)
        .collect();
        assert_eq!(rig.clips(2)[0].start_tick, 512);
        assert!(edits.iter().all(Option::is_none), "{edits:?}");
    }

    #[test]
    fn a_move_that_arrives_with_the_lift_still_moves_the_clip() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        let to = CLIP + vec2(2.0 * BEAT_PT, 0.0);
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        rig.frame(vec![button(CLIP, true)]);
        rig.frame(vec![egui::Event::PointerMoved(CLIP + vec2(BEAT_PT, 0.0))]);
        assert_eq!(rig.clips(2)[0].start_tick, 640);
        // A touch: the last move, the lift and the pointer gone in one frame.
        let f = rig.frame(vec![
            egui::Event::PointerMoved(to),
            button(to, false),
            egui::Event::PointerGone,
        ]);
        assert_eq!(rig.clips(2)[0].start_tick, 768);
        assert_eq!(f.edit, Some(SongEdit::Final));
    }

    #[test]
    fn a_drag_that_starts_on_a_floating_button_moves_nothing_under_it() {
        let mut song = song_with_a_clip();
        // Two bars on the drums, under the MiniSynth clip's buttons.
        song.channels[1].lanes[0].add_clip(0, 8.0);
        let mut rig = Rig::new(song);
        rig.click(CLIP);
        let copy = rig.rect_of(Id::new(("clip_button", 0_usize))).unwrap();
        let edits = rig.drag(copy.center(), copy.center() + vec2(2.0 * BEAT_PT, 0.0));
        assert!(edits.iter().all(Option::is_none), "{edits:?}");
        assert_eq!(rig.clips(1)[0].start_tick, 0);
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 2,
                lane: 0,
                id: 1
            })
        );
        assert_eq!(rig.screen.scroll_x, 0.0);
    }

    #[test]
    fn a_clip_near_the_last_tick_scrolled_to_draws_without_overflow() {
        // A clip no song makes, but a file can hold: the beat lines over it lie past u32 ticks.
        let mut song = song_with_a_clip();
        let clip = &mut song.channels[2].lanes[0].clips[0];
        (clip.start_tick, clip.visible_len_beats) = (u32::MAX - 10_000, 1000.0);
        let mut rig = Rig::new(song);
        rig.screen.scroll_x = f64::from(u32::MAX / PPQ) + 100.0;
        rig.frame(vec![]);
    }

    #[test]
    fn a_narrow_selected_clip_still_moves_from_most_of_its_width() {
        let mut song = song_with_a_clip();
        // A beat long: 38.4 points, little more than a handle's 1 U zone.
        song.channels[2].lanes[0].clips[0].visible_len_beats = 1.0;
        let mut rig = Rig::new(song);
        let quarter = pos2(201.6 + 9.6, CLIP.y);
        rig.click(quarter);
        rig.drag(quarter, quarter + vec2(BEAT_PT, 0.0));
        let clip = &rig.clips(2)[0];
        assert_eq!((clip.start_tick, clip.visible_len_beats), (640, 1.0));
    }

    #[test]
    fn a_change_from_outside_mid_drag_ends_the_drag() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.current();
        let to = CLIP + vec2(2.0 * BEAT_PT, 0.0);
        rig.frame(vec![egui::Event::PointerMoved(CLIP)]);
        rig.frame(vec![button(CLIP, true)]);
        rig.frame(vec![egui::Event::PointerMoved(CLIP + vec2(BEAT_PT, 0.0))]);
        assert_eq!(rig.clips(2)[0].start_tick, 640);
        // An undo puts the clip back while the button is still down.
        rig.song.channels[2].lanes[0].clips[0].start_tick = 512;
        let edits: Vec<_> = [vec![egui::Event::PointerMoved(to)], vec![button(to, false)]]
            .into_iter()
            .map(|events| rig.frame(events).edit)
            .collect();
        assert_eq!(rig.clips(2)[0].start_tick, 512);
        assert_eq!(edits, [None, None]);
    }

    #[test]
    fn the_handles_of_the_selected_clip_resize_it() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        // The right handle sits 0.65 U (20.8 points) right of the clip's end (x 357.2).
        let end = pos2(357.2 + 20.8, CLIP.y);
        let edits = rig.drag(end, end + vec2(2.2 * BEAT_PT, 0.0));
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
        let clip = &rig.clips(2)[0];
        assert_eq!((clip.start_tick, clip.visible_len_beats), (512, 6.0));
        // The left one trims a beat off the start, the notes staying where they were.
        let start = pos2(201.6, CLIP.y);
        rig.drag(start, start + vec2(BEAT_PT, 0.0));
        let clip = &rig.clips(2)[0];
        assert_eq!(
            (clip.start_tick, clip.visible_len_beats, clip.offset_beats),
            (640, 5.0, 1.0)
        );
    }

    #[test]
    fn copy_snap_and_delete_edit_the_song_and_edit_opens_the_editor() {
        let button = |i: usize| Id::new(("clip_button", i));
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        rig.frame(vec![]);
        assert_eq!(rig.click_id(button(0)).0, Some(SongEdit::Final));
        let starts: Vec<_> = rig.clips(2).iter().map(|c| (c.id, c.start_tick)).collect();
        assert_eq!(starts, [(1, 512), (2, 1024)]);
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 2,
                lane: 0,
                id: 2
            })
        );
        // Snap puts the copy back on the beat grid.
        rig.song.channels[2].lanes[0].clips[1].start_tick = 1000;
        rig.frame(vec![]);
        assert_eq!(rig.click_id(button(2)).0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2)[1].start_tick, 1024);
        let copy = ClipKey {
            channel: 2,
            lane: 0,
            id: 2,
        };
        assert_eq!(rig.click_id(button(3)), (None, Some(Nav::EditClip(copy))));
        assert_eq!(rig.click_id(button(1)).0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2).iter().map(|c| c.id).collect::<Vec<_>>(), [1]);
        assert_eq!(rig.screen.clips.selected, None);
        // Without a selection there are no buttons.
        rig.frame(vec![]);
        assert_eq!(rig.rect_of(button(0)), None);
    }

    #[test]
    fn more_opens_the_clip_dropdown_whose_unlink_loop_edits_the_song() {
        let mut song = song_with_a_clip();
        song.channels[2].lanes[0].clips[0].visible_len_beats = 6.0;
        let mut rig = Rig::new(song);
        rig.click(CLIP);
        rig.frame(vec![]);
        rig.click_id(Id::new(("clip_button", 4_usize)));
        rig.frame(vec![]);
        let unlink = Id::new("clip_more").with((0_usize, 1_usize));
        assert_eq!(rig.click_id(unlink).0, Some(SongEdit::Final));
        let clip = &rig.clips(2)[0];
        assert_eq!((clip.loop_len_beats, clip.events.len()), (6.0, 3));
    }

    #[test]
    fn more_mutes_and_unmutes_the_clip_as_final_edits() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        let more = Id::new(("clip_button", 4_usize));
        let item = |row: usize| Id::new("clip_more").with((0_usize, row));
        rig.click_id(more);
        assert_eq!(rig.click_id(item(3)).0, Some(SongEdit::Final));
        assert!(rig.clips(2)[0].muted);
        rig.click_id(more);
        assert_eq!(rig.click_id(item(4)).0, Some(SongEdit::Final));
        assert!(!rig.clips(2)[0].muted);
    }

    /// A wheel turn of `delta` points with the pointer at `at`.
    fn wheel_at(rig: &mut Rig, at: Pos2, delta: Vec2) {
        rig.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }

    #[test]
    fn the_playlist_wheel_zooms_the_time_on_the_ruler_the_rows_on_the_headers_and_scrolls_on_the_grid()
     {
        // Zoom ← clamp(zoom·(1 − 0.001·dy)) about the pointer, egui's raw wheel delta.
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        song.channels[1].lanes[0].add_clip(60 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        let m = rig.frame(vec![]).m;
        rig.screen.scroll_y = 64.0;
        // The ruler: the time only, the beat under the pointer kept (dy 200: a beat 1/0.8 as wide).
        let at = pos2(400.0, 24.0);
        let before = rig.screen.axis(GRID_X, &m).tick(at.x);
        wheel_at(&mut rig, at, vec2(0.0, 200.0));
        assert!(
            (rig.screen.zoom - 1.25f32.log2()).abs() < 1e-5,
            "{}",
            rig.screen.zoom
        );
        assert!((rig.screen.axis(GRID_X, &m).tick(at.x) - before).abs() < 0.01);
        assert_eq!((rig.screen.scroll_y, rig.screen.row_u), (64.0, Some(2.0)));
        // The headers: the rows only, the row position under the pointer kept.
        let at = pos2(24.0, 300.0);
        let (x, zoom) = (rig.screen.scroll_x, rig.screen.zoom);
        let row_at = |rig: &Rig| (rig.screen.scroll_y + at.y - 48.0) / rig.screen.row_h(&m);
        let before = row_at(&rig);
        wheel_at(&mut rig, at, vec2(0.0, 200.0));
        assert_eq!(rig.screen.row_u, Some(2.5));
        assert!((row_at(&rig) - before).abs() < 1e-4);
        assert_eq!((rig.screen.scroll_x, rig.screen.zoom), (x, zoom));
        // Rows are 2 U … 100/24 U.
        wheel_at(&mut rig, at, vec2(0.0, 900.0));
        assert_eq!(rig.screen.row_u, Some(100.0 / 24.0));
        wheel_at(&mut rig, at, vec2(0.0, -2000.0));
        assert_eq!(rig.screen.row_u, Some(2.0));
        // The grid: both ways, the zooms as they were (a beat is 48 points now).
        let (x, y, zoom) = (rig.screen.scroll_x, rig.screen.scroll_y, rig.screen.zoom);
        wheel_at(&mut rig, pos2(600.0, 200.0), vec2(-48.0, -30.0));
        assert!(
            (rig.screen.scroll_x - x - 1.0).abs() < 1e-4,
            "{}",
            rig.screen.scroll_x - x
        );
        assert_eq!(rig.screen.scroll_y, y + 30.0);
        assert_eq!((rig.screen.zoom, rig.screen.row_u), (zoom, Some(2.0)));
    }

    #[test]
    fn the_playlists_time_zoom_is_d_over_48_to_25_d_u_a_beat() {
        // (0.01/U)·4/d … (12/U)·4/d beats a pixel; d = TDIV byte 0.
        for d in [4u8, 3] {
            let mut song = song_with_a_clip();
            song.time_sig.subdivision = d;
            let mut rig = Rig::new(song);
            let m = rig.frame(vec![]).m;
            for _ in 0..10 {
                wheel_at(&mut rig, pos2(400.0, 24.0), vec2(0.0, -5000.0));
            }
            let beat_u = rig.screen.axis(GRID_X, &m).beat_w / m.u(1.0);
            assert!(
                (beat_u - f32::from(d) / 48.0).abs() < 1e-4,
                "d {d}: {beat_u}"
            );
            for _ in 0..20 {
                wheel_at(&mut rig, pos2(400.0, 24.0), vec2(0.0, 900.0));
            }
            let beat_u = rig.screen.axis(GRID_X, &m).beat_w / m.u(1.0);
            assert!(
                (beat_u - 25.0 * f32::from(d)).abs() < 1e-2,
                "d {d}: {beat_u}"
            );
        }
    }

    #[test]
    fn a_songs_stored_row_zoom_applies_through_the_clamp() {
        // HEAD@296 zoom y is rows a pixel: 1/80 → 80 px = 2.5 U at U = 32.
        for (zoom_y, row_u) in [
            (1.0 / 80.0, 2.5),
            (0.02, 2.0),
            (0.001, 100.0 / 24.0),
            (0.0, 2.0),
        ] {
            let mut song = song_with_a_clip();
            song.file.playlist_view[3] = zoom_y;
            let rig = Rig::new(song);
            assert_eq!(rig.screen.row_u, Some(row_u), "{zoom_y}");
        }
    }

    #[test]
    fn a_marquee_drag_never_scrolls_the_playlist() {
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song);
        rig.screen.scroll_y = 64.0;
        // Held 0.4 s on empty space, then dragged 50 points up and left.
        let from = pos2(600.0, 250.0);
        let to = from - vec2(50.0, 50.0);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![egui::Event::PointerMoved(from - vec2(25.0, 25.0))],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]);
        assert_eq!((rig.screen.scroll_x, rig.screen.scroll_y), (0.0, 64.0));
    }

    #[test]
    fn a_group_dragged_past_the_song_start_keeps_its_shape() {
        // The drums' clip at beat 4 first in the group, the synth's at beat 1; 3 beats left stops at the song start for
        // the synth's, and the drums' keeps its 3 beats on it.
        let mut song = song_with_a_clip();
        song.channels[2].lanes[0].clips[0].start_tick = PPQ;
        song.channels[1].lanes[0].add_clip(4 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        rig.screen.clips.group = vec![
            ClipRef {
                channel: 1,
                lane: 0,
                id: 1,
            },
            SYNTH_CLIP,
        ];
        rig.frame(vec![]);
        let from = pos2(GRID_X + 5.0 * BEAT_PT, 144.0);
        let edits = rig.drag(from, from - vec2(3.0 * BEAT_PT, 0.0));
        assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
        assert_eq!(
            (rig.clips(1)[0].start_tick, rig.clips(2)[0].start_tick),
            (3 * PPQ, 0)
        );
    }

    #[test]
    fn the_playlist_view_is_stored_in_the_song_and_restored_from_it() {
        // The view (position, zooms) is stored in the song, HEAD@296 (scroll x in beats, scroll y in rows, zoom x in
        // beats a pixel, zoom y in rows a pixel).
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        song.channels[1].lanes[0].add_clip(60 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        let m = rig.frame(vec![]).m;
        wheel_at(&mut rig, pos2(24.0, 300.0), vec2(0.0, 200.0));
        wheel_at(&mut rig, pos2(400.0, 24.0), vec2(0.0, 300.0));
        wheel_at(&mut rig, pos2(600.0, 200.0), vec2(-90.0, -40.0));
        rig.frame(vec![]);
        let screen = &rig.screen;
        let (row_h, beat_w) = (screen.row_h(&m), screen.axis(0.0, &m).beat_w);
        let want = [
            screen.scroll_x,
            f64::from(screen.scroll_y / row_h),
            f64::from(1.0 / beat_w),
            f64::from(1.0 / row_h),
        ];
        assert!(want[0] > 0.0 && want[1] > 0.0, "{want:?}");
        assert_eq!(rig.song.file.playlist_view, want);
        // Saved and opened again: the same scroll and rows.
        let bytes = crate::flm::write(&rig.song).unwrap();
        let song = crate::flm::read(&bytes).unwrap();
        assert_eq!(song.file.playlist_view, want);
        let again = Rig::new(song);
        assert_eq!(again.screen.row_u, rig.screen.row_u);
        assert!((again.screen.scroll_x - rig.screen.scroll_x).abs() < 1e-9);
        assert!((again.screen.scroll_y - rig.screen.scroll_y).abs() < 1e-3);
    }

    #[test]
    fn a_songs_view_opens_zoom_y_scroll_y_zoom_x_scroll_x_and_writes_the_clamped_zooms_back() {
        // The stored playlist view, HEAD@296, device pixels at ppp 1. The song ends at beat 64 and has 9 rows; the
        // track area is 301 points tall, the grid 1120 wide (after the 1.5 U column and t).
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        // Rows 1000 px, a beat 2 px, scrolled to row 7 and beat 1.5.
        song.file.playlist_view = [1.5, 7.0, 0.5, 0.001];
        let mut rig = Rig::new(song);
        let m = rig.frame(vec![]).m;
        // 1, 2: zoom y clamps to 100/24 U rows; then scroll y clamps by those rows (9.84 rows, 2.26 shown): row 7 stays.
        assert_eq!(rig.screen.row_u, Some(100.0 / 24.0));
        assert!((rig.screen.scroll_y / rig.screen.row_h(&m) - 7.0).abs() < 1e-4);
        // 3, 4: zoom x clamps to 25·d U … d/48 U a beat and to the range/view cap: the whole range, 2·(64 + 1/128)
        // beats, over the grid; then scroll x clamps by that zoom, to 0.
        let range = 2.0 * (64.0 + 1.0 / 128.0);
        let beat_w = rig.screen.axis(GRID_X, &m).beat_w;
        assert!(
            (1120.0 / f64::from(beat_w) - range).abs() < 1e-3,
            "{beat_w}"
        );
        assert_eq!(rig.screen.scroll_x, 0.0);
        // The clamped zooms go back into the song at once (f32); the scrolls stay as stored until the view moves.
        let view = rig.song.file.playlist_view;
        assert_eq!(view[..3], [1.5, 7.0, f64::from((range / 1120.0) as f32)]);
        assert!((view[3] - 0.0075).abs() < 1e-9 && view[3] == f64::from(view[3] as f32));
        rig.frame(vec![]);
        assert_eq!(rig.song.file.playlist_view, view);
        // A default view: 0.01 beats a pixel (100 points a beat here), the song long enough for it.
        let mut song = song_with_a_clip();
        song.channels[0].lanes[0].add_clip(400 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        rig.screen.row_u = None;
        rig.song.file.playlist_view[2] = 0.01;
        assert_eq!(rig.frame(vec![]).m.u(1.0), 32.0);
        assert!((rig.screen.axis(GRID_X, &m).beat_w - 100.0).abs() < 1e-3);
    }

    #[test]
    fn follow_playhead_keeps_the_playhead_a_quarter_in_and_turns_the_one_finger_time_scroll_off() {
        // The song is 64 beats long, longer than the 1120/38.4 beats shown.
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song);
        let shown = 1120.0 / f64::from(BEAT_PT);
        rig.screen.playing = true;
        FOLLOW.set(Some(40 * u64::from(PPQ)));
        rig.frame(vec![]);
        assert!((rig.screen.scroll_x - (40.0 - shown / 4.0)).abs() < 1e-3);
        // One finger on empty space, sideways and down: the time stays on the playhead, the rows scroll.
        let (from, to) = (pos2(600.0, 300.0), pos2(300.0, 200.0));
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(pos2(450.0, 250.0))],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]);
        assert!((rig.screen.scroll_x - (40.0 - shown / 4.0)).abs() < 1e-3);
        assert_eq!(rig.screen.scroll_y, 100.0);
        // The playhead moves on, and the view with it.
        FOLLOW.set(Some(50 * u64::from(PPQ)));
        rig.frame(vec![]);
        let at = 50.0 - shown / 4.0;
        assert!((rig.screen.scroll_x - at).abs() < 1e-3);
        // Stopped: the view stays.
        rig.screen.playing = false;
        FOLLOW.set(Some(60 * u64::from(PPQ)));
        rig.frame(vec![]);
        assert!((rig.screen.scroll_x - at).abs() < 1e-3);
        // A song no longer than the view does not follow.
        let mut song = song_with_a_clip();
        song.channels[0].lanes[0].clips.clear();
        let mut rig = Rig::new(song);
        rig.screen.playing = true;
        FOLLOW.set(Some(6 * u64::from(PPQ)));
        rig.frame(vec![]);
        assert_eq!(rig.screen.scroll_x, 0.0);
        FOLLOW.set(None);
    }

    #[test]
    fn a_sideways_scroll_or_zoom_hides_clip_plus_and_the_region_plus_and_cancels_a_ruler_press() {
        let mut rig = Rig::new(song_with_a_clip());
        // `+Clip` on the current (drums) track's empty space.
        let drums = pos2(GRID_X + 10.3 * BEAT_PT, 144.0);
        rig.click(drums);
        assert!(rig.plus_shown());
        // The rows' zoom keeps it; a sideways wheel over the grid hides it.
        wheel_at(&mut rig, pos2(24.0, 300.0), vec2(0.0, 100.0));
        assert!(rig.plus_shown());
        wheel_at(&mut rig, pos2(600.0, 300.0), vec2(-40.0, 0.0));
        assert!(!rig.plus_shown());
        // The ruler's region `+` goes at a time zoom.
        let plus = Id::new("playlist_region").with("plus");
        rig.select(2.0, 6.0);
        assert!(rig.rect_of(plus).is_some());
        wheel_at(&mut rig, pos2(600.0, 24.0), vec2(0.0, 100.0));
        assert!(rig.rect_of(plus).is_none());
        // A ruler press dragged sideways before its long press scrolls the time and starts no range.
        rig.song.play_loop = None;
        let (x, at) = (rig.screen.scroll_x, ruler(10.0));
        rig.frames(vec![
            vec![egui::Event::PointerMoved(at)],
            vec![button(at, true)],
            vec![egui::Event::PointerMoved(at - vec2(60.0, 0.0))],
            vec![],
            vec![],
            vec![],
            vec![button(at - vec2(60.0, 0.0), false)],
        ]);
        assert_eq!(rig.song.play_loop, None);
        let m = rig.frame(vec![]).m;
        let beat_w = rig.screen.axis(GRID_X, &m).beat_w;
        assert!((rig.screen.scroll_x - x - f64::from(60.0 / beat_w)).abs() < 1e-4);
    }

    #[test]
    fn the_rows_scrollbar_shows_the_visible_share_and_its_drag_scrolls_them() {
        // 9 rows and the `+` room are 688 points, `h` of them show (the track area's height).
        let mut rig = Rig::new(song_with_channels(8));
        let h = rig.frame(vec![]).area.height();
        let bar = Id::new("track_scrollbar");
        let thumb = rig.rect_of(bar).expect("drawn");
        assert!((thumb.height() - h * h / 688.0).abs() < 1e-3, "{thumb:?}");
        assert_eq!((thumb.top(), thumb.center().x), (48.0, 1170.0 - 2.4));
        // A drag of 50 points moves the rows 50·688/h points; the thumb follows the finger.
        let (from, to) = (thumb.center(), thumb.center() + vec2(0.0, 50.0));
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]);
        assert!((rig.screen.scroll_y - 50.0 * 688.0 / h).abs() < 1e-3);
        assert!((rig.rect_of(bar).expect("drawn").top() - 98.0).abs() < 1e-3);
        // All rows showing (2 rows and the `+` room, 240 points): no bar.
        let mut rig = Rig::new(song_with_channels(1));
        assert_eq!(rig.rect_of(bar), None);
    }

    #[test]
    fn two_fingers_zoom_both_axes_on_the_grid_and_only_the_time_on_the_ruler() {
        // The first finger's zone decides; zoom ← zoom·|(d − Δ)/d| about the other finger.
        use egui::TouchPhase::{End, Move, Start};
        let touch = |id, phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        };
        let pinch = |rig: &mut Rig, first: Pos2| {
            let (b, c) = (first + vec2(200.0, 100.0), first + vec2(300.0, 150.0));
            rig.frames(vec![
                vec![egui::Event::PointerMoved(first)],
                vec![touch(1, Start, first), button(first, true)],
                vec![touch(2, Start, b)],
                vec![touch(2, Move, c)],
                vec![touch(2, End, c)],
                vec![
                    touch(1, End, first),
                    button(first, false),
                    egui::Event::PointerGone,
                ],
            ]);
        };
        let mut song = song_with_channels(8);
        song.channels[0].lanes[0].add_clip(60 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        let m = rig.frame(vec![]).m;
        // On empty grid: 200 → 300 points apart sideways and 100 → 150 down, both 1.5 times.
        pinch(&mut rig, pos2(500.0, 150.0));
        let beat_w = rig.screen.axis(GRID_X, &m).beat_w;
        assert!((beat_w - 1.5 * BEAT_PT).abs() < 1e-3, "{beat_w}");
        let row_u = rig.screen.row_u.expect("set");
        assert!((row_u - 3.0).abs() < 1e-5, "{row_u}");
        // On the ruler: the time only.
        pinch(&mut rig, pos2(500.0, 24.0));
        let beat_w = rig.screen.axis(GRID_X, &m).beat_w;
        assert!((beat_w - 2.25 * BEAT_PT).abs() < 1e-3, "{beat_w}");
        assert_eq!(rig.screen.row_u, Some(row_u));
    }

    #[test]
    fn the_open_clamps_each_scroll_by_its_already_clamped_zoom() {
        // The stored view (HEAD@296). y: 20 px rows clamp to 2 U; under them 10.75 rows scroll over 4.7 shown, so row 5
        // stays (the stored zoom would show every row and clamp it to 0).
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        // x: the song ends at beat 400; 0.5 beats a pixel clamps to zoomMax 0.375 (420.75 beats shown, max 379.3), so
        // beat 300 stays (the stored zoom would show 561 beats and clamp it to 239).
        song.channels[0].lanes[0].add_clip(396 * PPQ, 4.0);
        song.file.playlist_view = [300.0, 5.0, 0.5, 0.05];
        let mut rig = Rig::new(song);
        let m = rig.frame(vec![]).m;
        assert_eq!(rig.screen.row_u, Some(2.0));
        assert!(
            (rig.screen.scroll_y / rig.screen.row_h(&m) - 5.0).abs() < 1e-4,
            "{}",
            rig.screen.scroll_y
        );
        assert!(
            (rig.screen.scroll_x - 300.0).abs() < 1e-9,
            "{}",
            rig.screen.scroll_x
        );
    }

    #[test]
    fn the_stored_view_is_in_device_pixels() {
        // HEAD@296: no unit conversion. At 2 pixels a point, rows of 2.5·U_px pixels are 2.5 U, and 0.01 beats a pixel
        // is a beat of 50 points.
        let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE_INDEX);
        let mut screen = MainScreen::default();
        let mut stored = [0.0, 0.0, 0.01, 1.0 / f64::from(2.5 * m.u_px())];
        let area = Rect::from_min_max(pos2(0.0, 48.0), pos2(1170.0, 349.0));
        let content = Content {
            rows: 3,
            end: 64.0,
            d: 4,
        };
        screen.restore_view(&mut stored, area, content, &m);
        assert!((screen.row_u.expect("set") - 2.5).abs() < 1e-5);
        assert!((screen.axis(0.0, &m).beat_w - 50.0).abs() < 1e-3);
        // Written back in device pixels too.
        assert!(
            (stored[2] - 0.01).abs() < 1e-9
                && (stored[3] * f64::from(2.5 * m.u_px()) - 1.0).abs() < 1e-6
        );
    }

    #[test]
    fn only_a_song_without_clips_counts_one_bar_for_the_view() {
        // An empty song's range is 2·(1 bar + 1/128) beats, so its open caps the zoom there; a 2-beat song keeps its
        // own end, 2·(2 + 1/128).
        let beat_w = |song: Song| {
            let mut rig = Rig::new(song);
            let m = rig.frame(vec![]).m;
            rig.screen.axis(0.0, &m).beat_w
        };
        let empty = song_with_channels(1);
        assert!((1120.0 / beat_w(empty) - 2.0 * (4.0 + 1.0 / 128.0)).abs() < 1e-3);
        let mut short = song_with_channels(1);
        short.channels[1].lanes[0].add_clip(0, 2.0);
        assert!((1120.0 / beat_w(short) - 2.0 * (2.0 + 1.0 / 128.0)).abs() < 1e-3);
    }

    #[test]
    fn the_wheel_over_the_headers_does_not_zoom_under_a_held_header() {
        // A header held past its long press, not dragged.
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song);
        let at = pos2(20.0, 150.0);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(at)],
            vec![button(at, true)],
            vec![],
            vec![],
            vec![],
            vec![],
        ]);
        assert!(rig.screen.hold.is_some(), "held");
        let row_u = rig.screen.row_u;
        wheel_at(&mut rig, at, vec2(0.0, 200.0));
        assert_eq!(
            rig.screen.row_u, row_u,
            "the wheel zoomed the rows under a held header"
        );
    }

    #[test]
    fn a_fling_does_not_carry_into_the_next_song() {
        // A fling under way when the app opens another song stops with it.
        let mut song = song_with_a_clip();
        for _ in 0..6 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song.clone());
        let (from, to) = (pos2(600.0, 300.0), pos2(600.0, 200.0));
        rig.frames(vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(pos2(600.0, 250.0))],
            vec![button(to, false)],
        ]);
        // The app replaces the song (app.rs resets row_u, scroll_x): its stored view (row 0) applies.
        song.file.playlist_view = [0.0, 0.0, 1.0 / f64::from(BEAT_PT), 0.02];
        rig.song = song;
        rig.screen.row_u = None;
        rig.screen.scroll_x = 0.0;
        rig.frame(vec![]);
        assert_eq!(
            rig.screen.scroll_y, 0.0,
            "the old song's fling moved the new song's view"
        );
    }

    #[test]
    fn an_empty_lane_touch_calm_on_each_axis_is_a_tap_that_shows_clip_plus() {
        // 0.4 U both ways is over 0.5 U from the touch-down (Euclidean), but neither axis left its own calm zone: a
        // tap, which shows `+Clip` on the current (drums) track.
        let mut rig = Rig::new(song_with_a_clip());
        let at = pos2(48.0 + 10.3 * BEAT_PT, 144.0);
        let moved = at + vec2(12.8, 12.8);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(at)],
            vec![button(at, true)],
            vec![egui::Event::PointerMoved(moved)],
            vec![button(moved, false)],
        ]);
        assert!(rig.plus_shown());
    }

    #[test]
    fn a_row_zoom_keeps_the_rows_in_their_new_range() {
        // The rows' range counts the `+` room in points, so a row zoom shrinks it in rows: the zoom's own clamp came
        // before that, and the rows must not sit past the new end (nor spring back from it under the next press).
        let mut rig = Rig::new(song_with_a_clip());
        let f = rig.frame(vec![]);
        let (m, h) = (f.m, f.area.height());
        wheel_at(
            &mut rig,
            pos2(24.0, f.area.bottom() - 10.0),
            vec2(0.0, 1000.0 / 3.0),
        );
        let row_h = rig.screen.row_h(&m);
        assert!((row_h - 96.0).abs() < 1e-3, "{row_h}");
        let max = (3.0 * row_h + 112.0 - h).max(0.0);
        let at = rig.screen.scroll_y;
        assert!(at <= max + 1e-3, "{at} > {max}");
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        assert_eq!(rig.screen.scroll_y, at);
    }

    #[test]
    fn the_timeline_pans_and_zooms_and_the_clips_follow() {
        // A drum clip at beat 60: the song is 64 beats long, so the time scrolls over 0 … 128 beats.
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(60 * PPQ, 4.0);
        let mut rig = Rig::new(song);
        // A drag on empty space pans left by 100 points, 1:1 once out of the calm zone, its last move too slow to
        // fling; the clip is hit where it is drawn now.
        let at = |x: f32| vec![egui::Event::PointerMoved(pos2(x, 80.0))];
        rig.frames(vec![
            at(700.0),
            vec![button(pos2(700.0, 80.0), true)],
            at(650.0),
            at(602.0),
            at(600.0),
            vec![button(pos2(600.0, 80.0), false)],
        ]);
        assert!((rig.screen.scroll_x - 100.0 / f64::from(BEAT_PT)).abs() < 1e-4);
        rig.click(CLIP - vec2(100.0, 0.0));
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 2,
                lane: 0,
                id: 1
            })
        );
        // A pinch or Ctrl + wheel zooms around the pointer.
        let m = rig.frame(vec![]).m;
        let at = pos2(200.0, 80.0);
        let before = rig.screen.axis(GRID_X, &m).tick(at.x);
        rig.frame(vec![egui::Event::PointerMoved(at), egui::Event::Zoom(2.0)]);
        assert_eq!(rig.screen.zoom, 1.0);
        assert!((rig.screen.axis(GRID_X, &m).tick(at.x) - before).abs() < 0.01);
        // Shift + wheel scrolls sideways, no further than twice (the song's length + 1/128) less the beats shown.
        let shift_wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -50_000.0),
            modifiers: egui::Modifiers::SHIFT,
        };
        rig.frame(vec![egui::Event::PointerMoved(at), shift_wheel]);
        rig.frame(vec![]);
        let shown = f64::from((1170.0 - GRID_X) / rig.screen.axis(GRID_X, &m).beat_w);
        assert!(
            (rig.screen.scroll_x - (2.0 * (64.0 + 1.0 / 128.0) - shown)).abs() < 1e-3,
            "{}",
            rig.screen.scroll_x
        );
    }

    #[test]
    fn each_primary_track_is_followed_by_its_sub_tracks_unless_they_are_hidden() {
        let mut song = song_with_a_clip();
        song.channels[1].add_lane(LaneKind::Notes);
        song.channels[2].add_automation_lane(crate::song::AutomationTarget {
            instance_id: 0,
            param: 1,
        });
        let rows = |song: &Song| -> Vec<(usize, usize, u8)> {
            let rows = track_rows(song);
            rows.iter()
                .map(|r| (r.channel, r.lane, r.icon as u8))
                .collect()
        };
        // The primary shows its channel kind; a sub-track its own kind (automation as the master's Level track).
        assert_eq!(
            rows(&song),
            [(0, 0, 0), (1, 0, 1), (1, 1, 2), (2, 0, 2), (2, 1, 0)]
        );
        song.channels[1].zipped = true;
        assert_eq!(rows(&song), [(0, 0, 0), (1, 0, 1), (2, 0, 2), (2, 1, 0)]);
        assert!(
            track_rows(&song)
                .iter()
                .all(|r| r.hue == song.channels[r.channel].hue)
        );
    }

    /// A right click, which is how a long press reaches the headers.
    fn secondary_click(rig: &mut Rig, pos: Pos2) -> Frame {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        };
        rig.frame(vec![egui::Event::PointerMoved(pos)]);
        rig.frame(vec![button(true)]);
        rig.frame(vec![button(false)])
    }

    /// The centre of row `row`'s header (2 U rows below the 1.5 U ruler, U = 32).
    fn header_at(row: usize) -> Pos2 {
        pos2(24.0, 48.0 + (row as f32 + 0.5) * 64.0)
    }

    fn track_item(col: usize, row: usize) -> Id {
        Id::new("track_menu").with((col, row))
    }

    #[test]
    fn a_long_press_on_a_header_opens_its_track_menu_whose_pick_is_reported() {
        let mut rig = Rig::new(song_with_a_clip());
        // Not a click: the rack stays shut and no track is picked.
        let f = secondary_click(&mut rig, header_at(2));
        assert_eq!((f.nav, f.track), (None, None));
        // Column 1, row 2 of the MiniSynth's menu: `Add Track`.
        rig.frame(vec![]);
        let mut track = None;
        let rect = rig.rect_of(track_item(1, 2)).expect("the menu is open");
        for events in [
            vec![egui::Event::PointerMoved(rect.center())],
            vec![button(rect.center(), true)],
            vec![button(rect.center(), false)],
        ] {
            track = track.or(rig.frame(events).track);
        }
        assert_eq!(track, Some((2, 0, TrackOp::AddTrack)));
        rig.frame(vec![]);
        assert_eq!(rig.rect_of(track_item(1, 2)), None, "the menu closed");
        // A disabled item (`Delete Track`, the only notes track) picks nothing and keeps the menu open.
        secondary_click(&mut rig, header_at(2));
        // A disabled cell has no widget: it sits between `Insert Channel` and `Delete Channel`.
        let (above, below) = (track_item(0, 3), track_item(0, 5));
        let above = rig.rect_of(above).expect("the menu is open").center();
        let below = rig.rect_of(below).expect("the menu is open").center();
        let disabled = above + (below - above) / 2.0;
        let mut track = None;
        for events in [
            vec![egui::Event::PointerMoved(disabled)],
            vec![button(disabled, true)],
            vec![button(disabled, false)],
        ] {
            track = track.or(rig.frame(events).track);
        }
        assert_eq!(track, None);
        assert!(rig.rect_of(track_item(0, 5)).is_some(), "still open");
    }

    /// Presses `pos` with the mouse and holds it still, a frame every 0.1 s, until just before `secs`: the next
    /// frame lands at `secs` after the press.
    fn hold(rig: &mut Rig, pos: Pos2, secs: f64) {
        rig.frame(vec![egui::Event::PointerMoved(pos)]);
        rig.frame(vec![button(pos, true)]);
        let start = rig.time;
        while rig.time + 0.1 < start + secs - 1e-9 {
            rig.frame(vec![]);
        }
        rig.time = start + secs - 0.1;
    }

    // Track header gestures.

    #[test]
    fn a_mouse_held_still_300_ms_on_a_header_selects_and_its_release_opens_the_menu_there() {
        // The MiniSynth's row and the drums' row (the keyboard starts on the drums).
        for (row, channel) in [(2, 2), (1, 1)] {
            let mut rig = Rig::new(song_with_a_clip());
            rig.selected = 3 - channel;
            // 1.25 U to the right during the hold: sideways is calm, the zone being vertical only.
            let at = header_at(row) + vec2(40.0, 3.0);
            hold(&mut rig, header_at(row), 0.2);
            rig.frame(vec![egui::Event::PointerMoved(at)]);
            assert_eq!(rig.selected, 3 - channel, "not before the 300 ms");
            // The hold makes the track current and shows nothing.
            let f = rig.frame(vec![]);
            assert_eq!((f.nav, rig.selected), (None, channel));
            assert_eq!(rig.screen.track_menu, None);
            // Released there: the menu, centred on the release point.
            let f = rig.frame(vec![button(at, false)]);
            assert_eq!((f.nav, f.track, f.reorder), (None, None, None), "no click");
            assert_eq!(rig.screen.track_menu, Some((channel, 0, at)));
            assert!(rig.rect_of(track_item(0, 1)).is_some(), "Channel Settings");
        }
    }

    #[test]
    fn a_tap_on_a_header_selects_its_track_then_toggles_the_column() {
        let mut rig = Rig::new(song_with_a_clip());
        // Released at 250 ms: a tap. The MiniSynth's track becomes current; no rack, no menu.
        hold(&mut rig, header_at(2), 0.25);
        let f = rig.frame(vec![button(header_at(2), false)]);
        assert_eq!((f.nav, rig.selected), (None, 2));
        rig.frame(vec![]);
        assert_eq!(rig.screen.track_menu, None);
        assert_eq!(rig.screen.column, (0.0, 0.0));
        // A tap on the current track opens the column: c ← c + 0.4·(1 − c) a frame, snapping within 0.05.
        rig.click(header_at(2));
        let mut widths = Vec::new();
        for _ in 0..7 {
            let f = rig.frame(vec![]);
            widths.push(rig.screen.header_w(&f.m) / 32.0);
        }
        // From the frame after the tap: 0.4, 0.64, 0.784 …, snapping at 0.953 (within 0.05 of 1).
        let want = [0.4, 0.64, 0.784, 0.8704, 0.92224, 1.0, 1.0].map(|c: f32| 1.5 + 12.0 * c);
        for (w, want) in widths.iter().zip(want) {
            assert!((w - want).abs() < 1e-3, "{widths:?}");
        }
        // The grid starts t = 2 points after it.
        let f = rig.frame(vec![]);
        assert_eq!(f.grid_left, 13.5 * 32.0 + 2.0);
        // Two taps: no double-tap action, the second closes it again.
        rig.click(header_at(2));
        assert_eq!(rig.screen.column.1, 0.0);
        rig.click(header_at(1));
        assert_eq!(
            (rig.selected, rig.screen.column.1),
            (1, 0.0),
            "another track: selected"
        );
    }

    #[test]
    fn two_taps_on_the_masters_header_select_its_track_then_toggle_the_column() {
        // The master never takes the keyboard, but its track becomes current all the same.
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(header_at(0));
        rig.frame(vec![]);
        assert_eq!((rig.selected, rig.screen.column.1), (1, 0.0));
        rig.click(header_at(0));
        assert_eq!((rig.selected, rig.screen.column.1), (1, 1.0));
        // Another track: selected, the column stays.
        rig.click(header_at(2));
        assert_eq!((rig.selected, rig.screen.column.1), (2, 1.0));
    }

    #[test]
    fn half_a_u_down_before_the_hold_scrolls_the_playlist_instead() {
        // Nine channels: the rows scroll.
        let mut song = song_with_a_clip();
        for _ in 0..7 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song);
        rig.screen.scroll_y = 64.0;
        let from = header_at(3);
        rig.frame(vec![egui::Event::PointerMoved(from)]);
        rig.frame(vec![button(from, true)]);
        // 0.45 U down: still calm, and the playlist holds still.
        rig.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 14.4))]);
        assert_eq!(rig.screen.scroll_y, 64.0);
        // 0.55 U: the playlist takes the touch.
        rig.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 17.6))]);
        rig.frame(vec![egui::Event::PointerMoved(from + vec2(0.0, 30.0))]);
        assert!(rig.screen.scroll_y < 64.0, "{}", rig.screen.scroll_y);
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        let f = rig.frame(vec![button(from + vec2(0.0, 30.0), false)]);
        assert_eq!((f.nav, f.reorder), (None, None));
        assert_eq!(rig.screen.track_menu, None);
        assert_eq!(rig.selected, 1, "nothing selected");
    }

    /// The song of [`song_with_a_clip`] and another MiniSynth: four one-row channels.
    fn four_rows() -> Rig {
        let mut song = song_with_a_clip();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        Rig::new(song)
    }

    // Track header reorder and secondary tap.

    #[test]
    fn a_hold_then_a_vertical_drag_swaps_the_channel_live_and_drops_it_on_release() {
        let mut rig = four_rows();
        hold(&mut rig, header_at(1), 0.3);
        rig.frame(vec![]);
        assert_eq!(rig.selected, 1);
        // Two rows down at once: the Drums and the channel in row 3 swap; row 2 stays. Live: nothing is reported, and
        // the playlist does not scroll.
        let to = header_at(3);
        let f = rig.frame(vec![egui::Event::PointerMoved(to)]);
        assert_eq!(f.reorder, None);
        assert_eq!(rig.screen.scroll_y, 0.0);
        // Up past the top: the target is row 0, so the master swaps with it, to the Drums' old row 3.
        let up = header_at(0) - vec2(0.0, 40.0);
        rig.frame(vec![egui::Event::PointerMoved(up)]);
        rig.frame(vec![egui::Event::PointerMoved(to)]);
        let f = rig.frame(vec![button(to, false)]);
        assert_eq!(f.reorder, Some(vec![0, 3, 2, 1]), "dropped");
        rig.frame(vec![]);
        assert_eq!(rig.screen.track_menu, None, "no menu after a drag");
        // The master's row drags like any primary row.
        hold(&mut rig, header_at(0), 0.3);
        rig.frame(vec![]);
        rig.frame(vec![egui::Event::PointerMoved(header_at(2))]);
        let f = rig.frame(vec![button(header_at(2), false)]);
        assert_eq!(f.reorder, Some(vec![2, 1, 0, 3]));
    }

    #[test]
    fn the_target_row_is_the_start_row_plus_the_travel_rounded() {
        // Pressed low in row 1 (start row 1.9, floor 1): 0.4 rows down rounds back to row 1, though the finger is in
        // row 2; 0.6 rows rounds to row 2.
        for (travel, want) in [(0.4, None), (0.6, Some(vec![0, 2, 1, 3]))] {
            let mut rig = four_rows();
            let at = header_at(1) + vec2(0.0, 25.6);
            hold(&mut rig, at, 0.3);
            rig.frame(vec![]);
            let to = at + vec2(0.0, travel * 64.0);
            rig.frame(vec![egui::Event::PointerMoved(to)]);
            let f = rig.frame(vec![button(to, false)]);
            assert_eq!(f.reorder, want, "{travel} rows");
        }
    }

    #[test]
    fn the_dragged_row_follows_the_finger_with_a_rubber_band_past_the_ends_and_keeps_its_header_id()
    {
        let mut rig = four_rows();
        // Where each track row's header ends (its top may be clipped), two frames on (`read_response` gives the pass
        // before the last).
        let bottoms = |rig: &mut Rig| -> Vec<f32> {
            rig.frame(vec![]);
            rig.frame(vec![]);
            let rect = |i: usize| rig.ctx.read_response(Id::new(("track_header", i)));
            (0..4)
                .map(|i| rect(i).expect("drawn").rect.bottom())
                .collect()
        };
        hold(&mut rig, header_at(1), 0.3);
        rig.frame(vec![]);
        // A row down: the Drums swap with row 2, whose header shows in row 1.
        rig.frame(vec![egui::Event::PointerMoved(header_at(2))]);
        assert_eq!(bottoms(&mut rig), [112.0, 240.0, 176.0, 304.0]);
        // A quarter row on: the Drums' header with the finger, the rest in their rows.
        rig.frame(vec![egui::Event::PointerMoved(
            header_at(2) + vec2(0.0, 16.0),
        )]);
        assert_eq!(bottoms(&mut rig), [112.0, 256.0, 176.0, 304.0]);
        // 2.5 rows past the top (row −2.5): it shows at 0.05 of that, row −0.125, and the master swaps to its row 2.
        rig.frame(vec![egui::Event::PointerMoved(
            header_at(1) - vec2(0.0, 3.5 * 64.0),
        )]);
        assert_eq!(bottoms(&mut rig), [240.0, 104.0, 176.0, 304.0]);
        // The screen alone moves no song: after the drop the rows are as the song has them.
        rig.frame(vec![button(header_at(1), false)]);
        assert_eq!(bottoms(&mut rig), [112.0, 176.0, 240.0, 304.0]);
    }

    /// Ten one-row channels, row 1's header held 300 ms and dragged off its calm zone and back: a drag in place.
    fn ten_rows_dragging() -> Rig {
        let mut song = song_with_a_clip();
        for _ in 0..7 {
            song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        }
        let mut rig = Rig::new(song);
        hold(&mut rig, header_at(1), 0.3);
        rig.frame(vec![]);
        rig.frame(vec![egui::Event::PointerMoved(
            header_at(1) + vec2(0.0, 20.0),
        )]);
        rig.frame(vec![egui::Event::PointerMoved(header_at(1))]);
        rig
    }

    #[test]
    fn a_scroll_under_a_still_finger_moves_the_target_row_with_it() {
        // The mouse goes sideways onto the grid (the calm zone is vertical only) and its wheel scrolls a row on.
        let mut rig = ten_rows_dragging();
        let on_grid = pos2(600.0, header_at(1).y);
        wheel_at(&mut rig, on_grid, vec2(0.0, -64.0));
        assert_eq!(rig.screen.scroll_y, 64.0);
        let f = rig.frame(vec![button(on_grid, false)]);
        assert_eq!(f.reorder, Some(vec![0, 2, 1, 3, 4, 5, 6, 7, 8, 9]));
    }

    #[test]
    fn the_wheel_over_the_headers_does_not_zoom_the_rows_under_a_header_drag() {
        // The zoom rescaled the rows under the start row and scroll measured at the old height, so a still finger
        // dropped the track a row on.
        let mut rig = ten_rows_dragging();
        let row_u = rig.screen.row_u;
        wheel_at(&mut rig, header_at(1), vec2(0.0, 500.0));
        assert_eq!(rig.screen.row_u, row_u);
        let f = rig.frame(vec![button(header_at(1), false)]);
        assert_eq!(f.reorder, None);
        // Without a hold it zooms.
        wheel_at(&mut rig, header_at(1), vec2(0.0, 500.0));
        assert_ne!(rig.screen.row_u, row_u);
    }

    #[test]
    fn a_sub_track_row_never_starts_a_drag_and_its_release_opens_the_menu() {
        let mut song = song_with_a_clip();
        song.channels[1].add_lane(LaneKind::Drums);
        let mut rig = Rig::new(song);
        hold(&mut rig, header_at(2), 0.3);
        rig.frame(vec![]);
        let to = header_at(3);
        rig.frame(vec![egui::Event::PointerMoved(to)]);
        let f = rig.frame(vec![button(to, false)]);
        assert_eq!(f.reorder, None);
        assert_eq!(rig.screen.track_menu, Some((1, 1, to)));
    }

    #[test]
    fn a_secondary_click_selects_the_track_and_opens_its_menu_there() {
        let mut rig = four_rows();
        assert_eq!(rig.selected, 1);
        let at = header_at(3) + vec2(4.0, 6.0);
        secondary_click(&mut rig, at);
        assert_eq!(rig.selected, 3);
        assert_eq!(rig.screen.track_menu, Some((3, 0, at)));
    }

    #[test]
    fn a_drag_step_swaps_the_channel_whose_swap_lands_the_dragged_primary_nearest_the_target() {
        // One-row channels: the target row's own channel, however far: a swap, not an insert.
        let mut order = vec![0, 1, 2, 3, 4];
        assert_eq!(drag_step(&mut order, &[1; 5], 1, 4.0), 4.0);
        assert_eq!(order, [0, 4, 2, 3, 1]);
        // Rows: master 0, A 1-2, B 3, C 4-6.
        let counts = [1, 2, 1, 3];
        let mut order = vec![0, 1, 2, 3];
        let mut step = |raw: f32| {
            let at = drag_step(&mut order, &counts, 1, raw);
            (order.clone(), at)
        };
        // A row down, A's primary lands in row 2 swapped with B; it stays there under a still finger.
        assert_eq!(step(2.0), (vec![0, 2, 1, 3], 2.0));
        assert_eq!(step(2.0).0, [0, 2, 1, 3]);
        // In row 3 a swap with C (landing in row 5) is no nearer; in row 4 it is, and a still finger keeps it.
        assert_eq!(step(3.0).0, [0, 2, 1, 3]);
        assert_eq!(step(4.0).0, [0, 2, 3, 1]);
        assert_eq!(step(4.0).0, [0, 2, 3, 1]);
        // The last row for A is 5 (7 rows, less its extra track): past it, 0.05 of the finger.
        let (order, at) = step(7.0);
        assert_eq!(order, [0, 2, 3, 1]);
        assert!((at - 5.1).abs() < 1e-6, "{at}");
        // Past the top, the master swaps down to A's place.
        assert_eq!(step(-1.0), (vec![1, 2, 3, 0], -0.05));
    }

    #[test]
    fn insert_channel_shows_the_add_channel_popup_whose_pick_goes_in_at_the_track() {
        let mut rig = Rig::new(song_with_a_clip());
        secondary_click(&mut rig, header_at(1));
        rig.frame(vec![]);
        let rect = rig.rect_of(track_item(0, 3)).expect("Insert Channel");
        let mut track = None;
        for events in [
            vec![egui::Event::PointerMoved(rect.center())],
            vec![button(rect.center(), true)],
            vec![button(rect.center(), false)],
        ] {
            track = track.or(rig.frame(events).track);
        }
        assert_eq!(track, None, "the screen handles it");
        // `Instrument`, column 0 row 2 of the add-channel popup.
        let item = Id::new("insert_channel_popup").with((0_usize, 2_usize));
        let rect = rig.rect_of(item).expect("the add-channel popup is open");
        let mut insert = None;
        for events in [
            vec![egui::Event::PointerMoved(rect.center())],
            vec![button(rect.center(), true)],
            vec![button(rect.center(), false)],
        ] {
            insert = insert.or(rig.frame(events).insert);
        }
        assert_eq!(insert, Some((1, ChannelKind::Instrument)));
        rig.frame(vec![]);
        assert_eq!(rig.rect_of(item), None);
    }

    #[test]
    fn a_sub_track_row_shows_and_edits_its_own_clips_and_menu() {
        let mut song = song_with_a_clip();
        let lane = song.channels[2].add_lane(LaneKind::Notes);
        song.channels[2].lanes[lane].add_clip(0, 4.0);
        let mut rig = Rig::new(song);
        // Row 3 is the MiniSynth's second notes track: its clip (beats 0 to 4) selects there.
        let clip = pos2(GRID_X + 2.0 * BEAT_PT, header_at(3).y);
        rig.click(clip);
        assert_eq!(
            rig.screen.clips.selected,
            Some(ClipRef {
                channel: 2,
                lane: 1,
                id: 1
            })
        );
        // A tap beside it clears the selection; on that (current) track the next shows `+Clip`, which adds a bar.
        let empty = pos2(GRID_X + 10.0 * BEAT_PT, header_at(3).y);
        rig.click(empty);
        rig.click(empty);
        assert_eq!(rig.plus().0, Some(SongEdit::Final));
        assert_eq!(rig.song.channels[2].lanes[1].clips.len(), 2);
        assert_eq!(rig.clips(2).len(), 1, "the primary's are untouched");
        // Its menu is the sub-track's: `Delete Track` is enabled (two notes tracks).
        secondary_click(&mut rig, header_at(3));
        rig.frame(vec![]);
        let mut track = None;
        let rect = rig.rect_of(track_item(0, 4)).expect("the menu is open");
        for events in [
            vec![egui::Event::PointerMoved(rect.center())],
            vec![button(rect.center(), true)],
            vec![button(rect.center(), false)],
        ] {
            track = track.or(rig.frame(events).track);
        }
        assert_eq!(track, Some((2, 1, TrackOp::DeleteTrack)));
    }

    // ---- the keymap's editing and zoom actions (`ui/edit_keys.rs`) ----

    impl Rig {
        /// A press of `key` with `mods` and its release, in one frame; the frame's song edit.
        fn key(&mut self, key: egui::Key, mods: egui::Modifiers) -> Option<SongEdit> {
            self.frame(crate::ui::edit_keys::tests::tap(key, mods).into())
                .edit
        }

        fn starts(&self, channel: usize) -> Vec<(u32, u32)> {
            (self.clips(channel).iter())
                .map(|c| (c.id, c.start_tick))
                .collect()
        }
    }

    #[test]
    fn the_editing_keys_copy_cut_delete_and_paste_the_selected_clips_at_the_touched_beat() {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let mut rig = Rig::new(song_with_a_clip());
        // The touch on the clip, at beat 6, is the grid cursor.
        rig.click(CLIP);
        assert_eq!(rig.key(Key::C, cmd), None);
        assert_eq!(rig.key(Key::V, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(1, 512), (2, 768)]);
        let pasted = ClipRef {
            id: 2,
            ..SYNTH_CLIP
        };
        assert_eq!(rig.screen.clips.selected, Some(pasted));
        assert_eq!(rig.key(Key::Delete, Modifiers::NONE), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(1, 512)]);
        assert_eq!(rig.screen.clips.selected, None);
        // Nothing selected: nothing to delete.
        assert_eq!(rig.key(Key::Delete, Modifiers::NONE), None);
        // Cut: on the clipboard and gone; paste puts it back at the cursor.
        rig.click(CLIP);
        assert_eq!(rig.key(Key::X, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), []);
        assert_eq!(rig.key(Key::V, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(3, 768)]);
    }

    #[test]
    fn escape_deselects_and_enter_selects_the_clip_at_the_cursor_or_adds_one() {
        use egui::{Key, Modifiers};
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        assert_eq!(rig.key(Key::Escape, Modifiers::NONE), None);
        assert_eq!(rig.screen.clips.selected, None);
        // `Select clip`: the clip under the cursor, current and selected, no edit.
        rig.song.channels[2].lanes[0].file.current_clip_id = 0;
        assert_eq!(rig.key(Key::Enter, Modifiers::NONE), None);
        assert_eq!(rig.screen.clips.selected, Some(SYNTH_CLIP));
        assert_eq!(rig.current_clip(), Some(SYNTH_CLIP));
        assert_eq!(rig.starts(2), [(1, 512)]);
        // With no clip there, `Add clip`: a bar on the cursor's track from its beat, current and selected.
        assert_eq!(rig.key(Key::Delete, Modifiers::NONE), Some(SongEdit::Final));
        assert_eq!(rig.key(Key::Enter, Modifiers::NONE), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(2, 768)]);
        assert_eq!(rig.clips(2)[0].loop_len_beats, 4.0);
        let added = ClipRef {
            id: 2,
            ..SYNTH_CLIP
        };
        assert_eq!(rig.screen.clips.selected, Some(added));
    }

    #[test]
    fn a_lone_clip_pastes_onto_the_touched_track_of_its_kind() {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let mut song = song_with_a_clip();
        song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
        song.channels[3].lanes[0].add_clip(0, 16.0);
        let mut rig = Rig::new(song);
        rig.click(CLIP);
        rig.key(Key::C, cmd);
        // The other synth's clip, a row lower at beat 6: the cursor goes to its track.
        rig.click(CLIP + vec2(0.0, 64.0));
        assert_eq!(rig.key(Key::V, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(3), [(1, 0), (2, 768)]);
        assert_eq!(rig.starts(2), [(1, 512)]);
    }

    #[test]
    fn the_cursor_and_the_clip_clipboard_follow_a_reorder_and_a_lane_insert() {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let mut song = song_with_a_clip();
        song.channels[2].lanes[0].add_clip(1536, 4.0);
        let mut rig = Rig::new(song);
        // The cursor on the synth's track at beat 6; both its clips (a group) on the clipboard.
        rig.click(CLIP);
        let second = ClipRef {
            id: 2,
            ..SYNTH_CLIP
        };
        rig.screen.clips.selected = None;
        rig.screen.clips.group = vec![SYNTH_CLIP, second];
        rig.key(Key::C, cmd);
        // A reorder: the synth (2) and the drums (1) swap, as the app re-keys the playlist after one.
        rig.song.channels.swap(1, 2);
        let swap = |c: usize| match c {
            1 => 2,
            2 => 1,
            c => c,
        };
        rig.screen.clips.rekey(|c, l| Some((swap(c), l)));
        // Two clips go back on their own track, which moved with the clipboard: the earliest on the cursor.
        assert_eq!(rig.key(Key::V, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(1), [(1, 512), (2, 1536), (3, 768), (4, 1792)]);
        // A lane inserted above the synth's notes lane: its track is lane 1 now, and the cursor follows it there.
        let mut synth = rig.song.channels.remove(1);
        synth
            .lanes
            .insert(0, crate::song::Lane::new(LaneKind::Notes));
        rig.song.channels.insert(1, synth);
        rig.screen
            .clips
            .rekey(|c, l| Some((c, if c == 1 { l + 1 } else { l })));
        rig.key(Key::Escape, Modifiers::NONE);
        assert_eq!(rig.key(Key::Enter, Modifiers::NONE), None, "Select clip");
        let under = ClipRef {
            channel: 1,
            lane: 1,
            id: 1,
        };
        assert_eq!(rig.screen.clips.selected, Some(under));
        assert!(rig.song.channels[1].lanes[0].clips.is_empty());
        // Its track gone, the clip goes from the clipboard: nothing to paste.
        rig.screen
            .clips
            .rekey(|c, l| ((c, l) != (1, 1)).then_some((c, l)));
        assert_eq!(rig.key(Key::V, cmd), None);
    }

    #[test]
    fn the_editing_keys_paste_and_add_clips_on_the_snap_modes_grid() {
        use crate::ui::piano_roll::SnapMode;
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let mut rig = Rig::new(song_with_a_clip());
        // The cursor a hair before beat 6: the beat grid puts a paste on beat 6 (768), the bar grid on beat 4 (512).
        rig.click(CLIP);
        rig.key(Key::C, cmd);
        rig.screen.snap.mode = SnapMode::Bar;
        assert_eq!(rig.key(Key::V, cmd), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(1, 512), (2, 512)]);
        // Add clip with no clip at the cursor (both gone): on beat 4 too.
        rig.screen.clips.selected = Some(SYNTH_CLIP);
        rig.key(Key::Delete, Modifiers::NONE);
        rig.screen.clips.selected = Some(ClipRef {
            id: 2,
            ..SYNTH_CLIP
        });
        rig.key(Key::Delete, Modifiers::NONE);
        assert_eq!(rig.key(Key::Enter, Modifiers::NONE), Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(3, 512)]);
    }

    // The left column, the mini mixer (expanded track headers) and the playhead line.

    /// The rig with its column fully expanded (c = 1).
    fn expanded(song: Song) -> Rig {
        let mut rig = Rig::new(song);
        rig.screen.column = (1.0, 1.0);
        rig.frame(vec![]);
        rig
    }

    fn texts(f: &Frame) -> Vec<(String, Color32, f32, Rect)> {
        (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => {
                    let format = &t.galley.job.sections[0].format;
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    Some((
                        t.galley.text().to_owned(),
                        format.color,
                        format.font_id.size,
                        rect,
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// The filled rects of `f` in paint order, groups flattened.
    fn rects(f: &Frame) -> Vec<(Rect, Color32)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(Rect, Color32)>) {
            match shape {
                egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        f.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
        out
    }

    fn circles(f: &Frame) -> Vec<(Pos2, f32, Color32)> {
        (f.shapes.iter())
            .filter_map(|s| match &s.shape {
                egui::Shape::Circle(c) => Some((c.center, c.radius, c.fill)),
                _ => None,
            })
            .collect()
    }

    fn theme_colors() -> crate::theme::ThemeColors {
        theme_by_id(0).colors
    }

    #[test]
    fn the_time_ruler_and_the_grid_start_t_after_the_column_on_a_background_ruler_row() {
        // U = 32: t = 2 points.
        let mut rig = Rig::new(song_with_a_clip());
        let f = rig.frame(vec![]);
        assert_eq!(f.grid_left, 48.0 + 2.0);
        let c = theme_colors();
        let rects = rects(&f);
        // The ruler row is a `background` panel over the full width; the time ruler's fill, `background` too, starts
        // at the grid.
        assert!(rects.contains(&(
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1170.0, 48.0)),
            c.background.to_color32()
        )));
        assert!(rects.contains(&(
            Rect::from_min_max(pos2(50.0, 0.0), pos2(1170.0, 48.0)),
            c.background.to_color32()
        )));
        // The lanes too: the clip's lane (the MiniSynth's, not the current track: `controlFill`) starts at the grid.
        let lane = (rects.iter())
            .find(|(r, fill)| *fill == c.control_fill.to_color32() && r.top() == 48.0 + 128.0);
        assert_eq!(lane.map(|l| l.0.left()), Some(50.0));
        let f = expanded(song_with_a_clip()).frame(vec![]);
        assert_eq!(f.grid_left, 13.5 * 32.0 + 2.0);
    }

    #[test]
    fn the_snap_button_is_always_in_the_rulers_top_left_and_a_tap_toggles_snap() {
        let mut rig = Rig::new(song_with_a_clip());
        let c = theme_colors();
        // Rect (0, 0, 1.5 U, ruler h − 1 px): a disc of 0.325 · 47 under a 2 px `text` ring.
        let center = pos2(24.0, 23.5);
        let r = 0.325 * 47.0;
        let disc = |f: &Frame| -> Vec<(f32, Color32)> {
            let mut d: Vec<(f32, Color32)> = (circles(f).into_iter())
                .filter(|c| c.0 == center)
                .map(|c| (c.1, c.2))
                .collect();
            d.sort_by(|a, b| b.0.total_cmp(&a.0));
            d
        };
        let f = rig.frame(vec![]);
        assert_eq!(
            disc(&f),
            [(r + 2.0, c.text.to_color32()), (r, c.text.to_color32())],
            "on by default"
        );
        // Left of the playhead knob's square, which covers the rest of the cell at beat 0.
        rig.click(LEFT_OF_KNOB);
        rig.frame(vec![egui::Event::PointerMoved(pos2(600.0, 300.0))]);
        let f = rig.frame(vec![]);
        assert!(!rig.screen.snap.on);
        assert_eq!(
            disc(&f),
            [
                (r + 2.0, c.text.to_color32()),
                (r, c.background.to_color32())
            ]
        );
        // Expanded too.
        let mut rig = expanded(song_with_a_clip());
        let f = rig.frame(vec![]);
        assert_eq!(disc(&f).len(), 2);
        rig.click(center);
        assert!(!rig.screen.snap.on);
    }

    /// In the Snap button's cell, left of the playhead knob's square (1.35 × the 1.5 U ruler, centred on the grid's
    /// left at x 50: 17.6 … 82.4), which covers the rest of the cell at beat 0 and takes its touches.
    const LEFT_OF_KNOB: Pos2 = pos2(8.0, 23.5);

    #[test]
    fn a_long_press_on_snap_opens_the_snap_list_whose_pick_turns_snap_on() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.screen.snap.on = false;
        hold(&mut rig, LEFT_OF_KNOB, 0.4);
        rig.frame(vec![]);
        rig.frame(vec![button(LEFT_OF_KNOB, false)]);
        // The popup grows in first.
        for _ in 0..30 {
            rig.frame(vec![]);
        }
        let f = rig.frame(vec![]);
        let labels: Vec<String> = texts(&f).into_iter().map(|t| t.0).collect();
        for label in ["Auto", "1/4 Step", "Beat", "Bar"] {
            assert!(labels.contains(&label.to_owned()), "{label}: {labels:?}");
        }
        let at = texts(&f)
            .into_iter()
            .find(|t| t.0 == "Bar")
            .unwrap()
            .3
            .center();
        rig.click(at);
        assert_eq!(
            rig.screen.snap,
            crate::ui::piano_roll::notes::Snap {
                on: true,
                mode: crate::ui::piano_roll::notes::SnapMode::Bar
            }
        );
    }

    #[test]
    fn with_snap_off_a_ruler_tap_takes_the_tick_under_it() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.screen.snap.on = false;
        let at = pos2(50.0 + 3.4 * BEAT_PT, 24.0);
        rig.frame(vec![egui::Event::PointerMoved(at)]);
        rig.frame(vec![button(at, true)]);
        // 3.4 beats: tick 435 (whole ticks), not the beat line 384.
        assert_eq!(rig.frame(vec![button(at, false)]).playhead, Some(435));
    }

    #[test]
    fn at_c_1_the_song_name_is_12_pt_centred_over_the_column() {
        let mut rig = Rig::new(song_with_a_clip());
        assert!(
            !texts(&rig.frame(vec![]))
                .iter()
                .any(|t| t.0.starts_with("New Song"))
        );
        let mut rig = expanded(song_with_a_clip());
        let f = rig.frame(vec![]);
        let name = texts(&f)
            .into_iter()
            .find(|t| t.0 == "New Song")
            .expect("the song name");
        // 12 pt at U = 32: 24 px, `text`, centred in (2 U, 0, 13.5 U − 2 U, 1.5 U).
        assert_eq!((name.1, name.2), (theme_colors().text.to_color32(), 24.0));
        let want = Rect::from_min_size(pos2(64.0, 0.0), vec2(13.5 * 32.0 - 64.0, 48.0)).center();
        assert!((name.3.center() - want).length() < 1.0, "{:?}", name.3);
        // A song with a title shows it.
        rig.song.title = "Groove".into();
        assert!(texts(&rig.frame(vec![])).iter().any(|t| t.0 == "Groove"));
    }

    #[test]
    fn while_the_column_animates_no_control_or_label_shows() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(header_at(2));
        rig.click(header_at(2));
        let solo = |f: &Frame| texts(f).iter().filter(|t| t.0 == "Solo").count();
        let mut seen = Vec::new();
        for _ in 0..7 {
            let f = rig.frame(vec![]);
            seen.push((rig.screen.column.0, solo(&f)));
        }
        // Until c snaps to 1 none shows; from that frame on, one per channel but the master's.
        assert!(
            seen.iter().all(|&(c, n)| (c == 1.0) == (n == 2)),
            "{seen:?}"
        );
        assert_eq!(seen.last().map(|s| s.1), Some(2));
        // And the first frame of a collapse hides them.
        rig.click(header_at(2));
        assert_eq!(solo(&rig.frame(vec![])), 0);
    }

    #[test]
    fn at_c_1_only_the_header_part_takes_the_header_gestures_and_the_controls_edit_as_params() {
        let mut rig = expanded(song_with_a_clip());
        rig.selected = 2;
        rig.click(header_at(2));
        // That made row 2 current (it was the keyboard's channel already); now the mute cell (1.5 U … 3 U).
        rig.screen.column = (1.0, 1.0);
        let mute = pos2(2.25 * 32.0, header_at(2).y);
        let (edit, _) = rig.click(mute);
        assert_eq!(edit, Some(SongEdit::ChannelParam));
        assert_eq!(rig.song.channels[2].params.0[2], 1.0);
        assert_eq!(rig.screen.column.1, 1.0, "not a header tap");
        // The control's touch is the app's recent control.
        assert_eq!(
            rig.screen.touch.map(|t| (t.0, t.1.target.param)),
            Some((2, 2))
        );
        // The icon cell is still the header: a tap on the current track collapses the column.
        rig.click(header_at(2));
        assert_eq!(rig.screen.column.1, 0.0);
    }

    #[test]
    fn a_held_header_grows_its_cell_by_the_knob_law_and_wobbles_back() {
        let mut rig = Rig::new(song_with_a_clip());
        rig.selected = 2;
        let ink = channel_color(rig.song.channels[2].hue, &theme_by_id(0)).to_color32();
        let cell = |f: &Frame| {
            (rects(f).into_iter())
                .find(|&(r, fill)| fill == ink && r.width() == 48.0 && r.center().x == 24.0)
                .map(|r| r.0.height())
        };
        hold(&mut rig, header_at(2), 0.4);
        let mut heights = Vec::new();
        for _ in 0..4 {
            heights.push(cell(&rig.frame(vec![])).unwrap());
        }
        // Held: 63·(1 + (1 − max(0.01, q))·10/63), q = 0.6 a sixtieth of a second from the hold (the knobs' own law,
        // `widgets::pulse`): with the rig's 0.1 s frames, 1 − 0.6⁷ of the 10 px a frame after it, then 0.99 of them.
        let grown = |k: f32| 63.0 + k * 10.0;
        assert!(
            (heights[0] - grown(1.0 - 0.6f32.powi(7))).abs() < 1e-3,
            "{heights:?}"
        );
        assert!(
            heights[1..].iter().all(|h| (h - grown(0.99)).abs() < 1e-3),
            "{heights:?}"
        );
        rig.frame(vec![button(header_at(2), false)]);
        let mut back = Vec::new();
        for _ in 0..12 {
            back.push(cell(&rig.frame(vec![])).unwrap());
        }
        // Let go: it wobbles (overshoots below 63) and settles at 63.
        assert!(back.iter().any(|&h| h < 63.0), "{back:?}");
        assert_eq!(back.last(), Some(&63.0));
    }

    #[test]
    fn the_playhead_line_shows_stopped_under_the_header_column_and_the_disc_overhangs_the_grid() {
        // No output: the playhead is at tick 0, the grid's left edge (x 50). What shows of each rect: its part inside
        // its clip. At beat 0 the line's left half falls in the t gap and shows (x 48 … 52); the header column covers
        // only the shadow's left pixel, left of its right edge at x 48.
        let mut rig = Rig::new(song_with_a_clip());
        let f = rig.frame(vec![]);
        let c = theme_colors();
        let mut shapes: Vec<(Rect, Color32)> = Vec::new();
        for s in &f.shapes {
            let group = match &s.shape {
                egui::Shape::Vec(v) => v.clone(),
                shape => vec![shape.clone()],
            };
            for shape in group {
                if let egui::Shape::Rect(r) = shape {
                    shapes.push((r.rect.intersect(s.clip_rect), r.fill));
                }
            }
        }
        let tracks = f.area.y_range();
        let line = (shapes.iter()).position(|&(r, fill)| {
            fill == c.text.to_color32() && r == Rect::from_x_y_ranges(48.0..=52.0, tracks)
        });
        let shadow = (shapes.iter()).position(|&(r, fill)| {
            fill == c.background.to_color32().gamma_multiply(0.3)
                && r == Rect::from_x_y_ranges(48.0..=53.0, tracks)
        });
        let header = (shapes.iter()).position(|&(r, fill)| {
            fill == c.module_fill.to_color32() && r.left() == 0.0 && r.top() == 48.0
        });
        let (line, shadow, header) = (
            line.expect("the line"),
            shadow.expect("its shadow"),
            header.expect("the master row"),
        );
        // Shadow, line, then the header column over them.
        assert!(shadow < line && line < header, "{shadow} {line} {header}");
        // The disc: 1.35 · 48 square on the ruler's middle at the playhead, radius 29.4, its play icon in `menuFill`.
        let disc = circles(&f)
            .into_iter()
            .find(|c| (c.1 - 29.4).abs() < 1e-3)
            .expect("the disc");
        assert_eq!((disc.0, disc.2), (pos2(50.0, 24.0), c.text.to_color32()));
        assert!(disc.0.y + disc.1 > 48.0, "it overhangs into the grid");
        // Armed: its icon is REC (a `menuFill` disc on it).
        rig.screen.armed = true;
        let f = rig.frame(vec![]);
        let rec = (circles(&f).into_iter())
            .any(|c| c.0.distance(pos2(50.0, 24.0)) < 1.0 && c.2 == c_menu_fill());
        assert!(rec);
    }

    #[test]
    fn the_playhead_line_lies_over_the_clips_and_under_the_selections_frame() {
        // The MiniSynth's clip (beats 4 to 8), selected; the playhead dropped on beat 5, inside it.
        let mut rig = Rig::new(song_with_a_clip());
        rig.click(CLIP);
        rig.screen.knob.dropped = Some((640, rig.time + 0.1));
        let f = rig.frame(vec![]);
        let text = theme_colors().text.to_color32();
        // Flattened in paint order (the line is a group of two rects).
        let mut order = Vec::new();
        for s in &f.shapes {
            match &s.shape {
                egui::Shape::Vec(v) => order.extend(v.iter().cloned()),
                shape => order.push(shape.clone()),
            }
        }
        let x = GRID_X + 5.0 * BEAT_PT;
        let line = (order.iter()).position(|s| matches!(s, egui::Shape::Rect(r) if r.fill == text && r.rect.width() == 4.0 && (r.rect.center().x - x).abs() < 1.0));
        // The selection's chrome is its pulsing box (beats 4 … 8 of row 2) and its handles; the light outline is the
        // current clip's, drawn with it.
        let sbox = Rect::from_min_max(
            pos2(GRID_X + 4.0 * BEAT_PT, 176.0),
            pos2(GRID_X + 8.0 * BEAT_PT, 240.0),
        );
        let frame =
            (order.iter()).position(|s| matches!(s, egui::Shape::Rect(r) if r.rect == sbox));
        let clip = (order.iter()).position(|s| matches!(s, egui::Shape::Rect(r) if r.rect.contains(CLIP) && r.fill != text && r.rect.width() < 200.0));
        let (line, frame, clip) = (
            line.expect("the line"),
            frame.expect("the frame"),
            clip.expect("the clip"),
        );
        assert!(clip < line && line < frame, "{clip} {line} {frame}");
    }

    /// The playlist's playhead line: the x of its 2t `text` rect down the track area, if drawn.
    fn line_x(f: &Frame) -> Option<f32> {
        let text = theme_colors().text.to_color32();
        (rects(f).into_iter())
            .find(|&(r, fill)| fill == text && r.width() == 4.0 && r.y_range() == f.area.y_range())
            .map(|(r, _)| r.center().x)
    }

    /// The knob's disc centre x, if drawn.
    fn disc_x(f: &Frame) -> Option<f32> {
        (circles(f).into_iter())
            .find(|c| c.0.y == 24.0 && c.1 > 20.0)
            .map(|c| c.0.x)
    }

    #[test]
    fn a_touch_lift_keeps_the_line_and_the_disc_on_the_dropped_beat() {
        // egui clears the latest position on the `PointerGone` that ends a touch in its release frame; the line and the
        // disc must not flash back to the transport's tick for that frame.
        let mut rig = Rig::new(song_with_a_clip());
        let (from, to) = (pos2(GRID_X, 24.0), GRID_X + 5.3 * BEAT_PT);
        rig.frame(vec![egui::Event::PointerMoved(from)]);
        rig.frame(vec![button(from, true)]);
        rig.frame(vec![egui::Event::PointerMoved(pos2(to - 10.0, 30.0))]);
        let f = rig.frame(vec![egui::Event::PointerMoved(pos2(to, 30.0))]);
        let held = (line_x(&f), disc_x(&f));
        assert_eq!(
            held,
            (Some(GRID_X + 5.0 * BEAT_PT), Some(GRID_X + 5.0 * BEAT_PT))
        );
        let f = rig.frame(vec![
            button(pos2(to, 30.0), false),
            egui::Event::PointerGone,
        ]);
        assert_eq!((line_x(&f), disc_x(&f)), held);
        assert_eq!(f.seek, Some(640));
    }

    #[test]
    fn while_the_knob_is_dragged_the_line_and_the_disc_follow_the_finger_on_the_snap_grid() {
        // Beat = the finger's, clamped to [view-left beat, song end], rounded to the snap unit (unrounded with Snap
        // off); the drop seeks there. No output: tick 0, x 50.
        let drag = |rig: &mut Rig, from: f32, to: f32| -> (Option<f32>, Option<f32>, Option<u64>) {
            let from = pos2(from, 24.0);
            rig.frame(vec![egui::Event::PointerMoved(from)]);
            rig.frame(vec![button(from, true)]);
            rig.frame(vec![egui::Event::PointerMoved(pos2(to - 10.0, 30.0))]);
            let f = rig.frame(vec![egui::Event::PointerMoved(pos2(to, 30.0))]);
            let shown = (line_x(&f), disc_x(&f));
            let seek = rig.frame(vec![button(pos2(to, 30.0), false)]).seek;
            // Let the dropped knob go back to the (absent) output's tick 0.
            rig.time += 1.0;
            rig.frame(vec![]);
            (shown.0, shown.1, seek)
        };
        let beat = |b: f32| GRID_X + b * BEAT_PT;
        // The song ends at the MiniSynth's clip end, beat 8 (tick 1024).
        let mut rig = Rig::new(song_with_a_clip());
        // Snap on (Auto: a beat at this zoom): beat 5.3 → 5.
        assert_eq!(
            drag(&mut rig, GRID_X, beat(5.3)),
            (Some(beat(5.0)), Some(beat(5.0)), Some(640))
        );
        // Snap off: unrounded, the tick under the finger.
        rig.screen.snap.on = false;
        let (line, disc, seek) = drag(&mut rig, GRID_X, beat(5.3));
        // Tick 678: the disc on it, the line on its nearest pixel.
        let x = GRID_X + 678.0 / 128.0 * BEAT_PT;
        assert_eq!((line, disc, seek), (Some(x.round()), Some(x), Some(678)));
        // Bar mode: to the nearest bar (4 beats).
        rig.screen.snap = crate::ui::piano_roll::notes::Snap {
            on: true,
            mode: crate::ui::piano_roll::notes::SnapMode::Bar,
        };
        let x = beat(4.0);
        assert_eq!(
            drag(&mut rig, GRID_X, beat(5.3)),
            (Some(x.round()), Some(x), Some(512))
        );
        // Past the song end: its end; scrolled, never left of the view-left beat.
        assert_eq!(drag(&mut rig, GRID_X, beat(9.9)).2, Some(1024));
        // A 64-beat song scrolled 2 beats, the playhead held at beat 3 (a drop the output has not reported yet):
        // dragged onto the header column, it stops at the view-left beat 2.
        let mut song = song_with_a_clip();
        song.channels[1].lanes[0].add_clip(0, 64.0);
        let mut rig = Rig::new(song);
        rig.screen.snap.on = false;
        rig.screen.scroll_x = 2.0;
        rig.screen.knob.dropped = Some((384, rig.time + 100.0));
        rig.frame(vec![]);
        let (line, _, seek) = drag(&mut rig, GRID_X + BEAT_PT, 20.0);
        assert_eq!((line, seek), (Some(GRID_X), Some(256)));
    }

    #[test]
    fn plus_clip_copy_and_the_floating_snap_take_the_snap_unit() {
        let bar = crate::ui::piano_roll::notes::Snap {
            on: true,
            mode: crate::ui::piano_roll::notes::SnapMode::Bar,
        };
        let button = |i: usize| Id::new(("clip_button", i));
        // The MiniSynth's clip shows 6 beats from beat 4: Copy rounds its length up to the unit, 2 bars.
        let mut song = song_with_a_clip();
        song.channels[2].lanes[0].clips[0].visible_len_beats = 6.0;
        let mut rig = Rig::new(song);
        rig.screen.snap = bar;
        rig.click(CLIP);
        rig.frame(vec![]);
        assert_eq!(rig.click_id(button(0)).0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2)[1].start_tick, 512 + 1024);
        // The floating Snap: to the nearest bar (900 → 1024; a beat would be 896).
        rig.song.channels[2].lanes[0].clips[1].start_tick = 900;
        rig.frame(vec![]);
        assert_eq!(rig.click_id(button(2)).0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2)[1].start_tick, 1024);
        // `+Clip` at beat 13.6: the nearest bar line, beat 12 (a beat would be 14).
        let mut rig = Rig::new(song_with_a_clip());
        rig.screen.snap = bar;
        rig.click(CLIP);
        let beside = pos2(GRID_X + 13.6 * BEAT_PT, CLIP.y);
        rig.click(beside);
        rig.click(beside);
        assert_eq!(rig.plus().0, Some(SongEdit::Final));
        assert_eq!(rig.clips(2)[1].start_tick, 12 * 128);
        // With Snap off a drag moves to the whole tick under the finger: 0.6 beat is 76.8 ticks.
        let mut rig = Rig::new(song_with_a_clip());
        rig.screen.snap.on = false;
        rig.current();
        rig.drag(CLIP, CLIP + vec2(0.6 * BEAT_PT, 0.0));
        assert_eq!(rig.clips(2)[0].start_tick, 512 + 77);
    }

    fn c_menu_fill() -> Color32 {
        theme_colors().menu_fill.to_color32()
    }

    // ---- touch `Paste` and a press held over a focus loss ----

    /// Empty space on the MiniSynth's track (row 2, y 176 … 239), at beat 12, on the row's middle.
    const EMPTY_2: Pos2 = pos2(GRID_X + 12.0 * BEAT_PT, 207.5);

    impl Rig {
        /// The `Paste` button's rect as a frame with no input draws it.
        fn paste(&mut self) -> Option<Rect> {
            self.frame(vec![]);
            self.rect_of(Id::new("clip_paste"))
        }

        fn right_click(&mut self, pos: Pos2) {
            let secondary = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frames(vec![
                vec![egui::Event::PointerMoved(pos)],
                vec![secondary(true)],
                vec![secondary(false)],
            ]);
        }
    }

    #[test]
    fn a_press_on_empty_space_held_over_a_focus_loss_is_no_tap() {
        // Item 4: `decide` ran before the stale guard, so the stale press was a tap that made the track current.
        let mut rig = Rig::new(song_with_a_clip());
        assert_eq!(rig.selected, 1);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(EMPTY_2)],
            vec![button(EMPTY_2, true)],
            vec![egui::Event::WindowFocused(false)],
            vec![egui::Event::WindowFocused(true)],
            vec![button(EMPTY_2, false)],
        ]);
        assert_eq!(rig.selected, 1, "no track made current");
        assert!(!rig.plus_shown());
    }

    #[test]
    fn a_long_press_on_empty_space_or_a_right_click_shows_paste_while_clips_are_copied() {
        // A stationary hold or right click offers one round Paste bubble at the finger.
        let mut rig = Rig::new(song_with_a_clip());
        rig.long_press(EMPTY_2);
        assert_eq!(rig.paste(), None, "nothing copied");
        rig.right_click(EMPTY_2);
        assert_eq!(rig.paste(), None, "nothing copied");
        // The clip copied, then deselected.
        rig.click(CLIP);
        rig.frame(vec![]);
        rig.click_id(Id::new(("clip_button", 0_usize)));
        rig.click(EMPTY_2 + vec2(BEAT_PT, 0.0));
        assert_eq!(rig.screen.clips.selected, None);
        let held_at = EMPTY_2 + vec2(BEAT_PT, 10.0);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(held_at)],
            vec![button(held_at, true)],
            vec![],
            vec![],
            vec![],
        ]);
        assert!(rig.paste().is_some(), "Paste appears before lifting");
        rig.frame(vec![button(held_at, false)]);
        let paste = rig.paste().expect("the Paste button");
        // The row's rect is y 176 … 239.
        assert_eq!(paste, Rect::from_center_size(held_at, Vec2::splat(44.8)));
        // Its tap pastes the clip on that track at the beat under it: one final edit, the copy selected.
        assert_eq!(rig.click(paste.center()).0, Some(SongEdit::Final));
        assert_eq!(rig.starts(2), [(1, 512), (2, 1024), (3, 1664)]);
        let pasted = ClipRef {
            id: 3,
            ..SYNTH_CLIP
        };
        assert_eq!(rig.screen.clips.selected, Some(pasted));
        assert_eq!(rig.paste(), None);
        // A right click on empty space shows it too; any other press hides it, and so does a sideways scroll.
        let far = EMPTY_2 + vec2(6.0 * BEAT_PT, 0.0);
        rig.click(far);
        rig.right_click(far);
        assert!(rig.paste().is_some());
        rig.click(far + vec2(0.0, -64.0));
        assert_eq!(rig.paste(), None);
        rig.right_click(far);
        assert!(rig.paste().is_some());
        wheel_at(&mut rig, pos2(600.0, 300.0), vec2(-40.0, 0.0));
        assert_eq!(rig.paste(), None);
        // A long press that dragged is a marquee only, and one held over a focus loss nothing.
        rig.frames(vec![
            vec![egui::Event::PointerMoved(far)],
            vec![button(far, true)],
            vec![],
            vec![],
            vec![],
        ]);
        let moved = rig.frame(vec![egui::Event::PointerMoved(far + vec2(60.0, 0.0))]);
        assert!(
            !moved.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "Paste")),
            "drag hides Paste in the same frame"
        );
        rig.frame(vec![button(far + vec2(60.0, 0.0), false)]);
        assert_eq!(rig.paste(), None);
        rig.frames(vec![
            vec![egui::Event::PointerMoved(far)],
            vec![button(far, true)],
            vec![],
            vec![],
            vec![],
            vec![egui::Event::WindowFocused(false)],
            vec![egui::Event::WindowFocused(true)],
            vec![button(far, false)],
        ]);
        assert_eq!(rig.paste(), None);
    }
}
