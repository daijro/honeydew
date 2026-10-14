//! The rack side panel: one channel's module cards under a channel-coloured header bar, each card's ≡ menu, the Mix /
//! Post Gain / Wet Pan dialog (an instrument's level popup), `Add Module`, the SENDS section and, on an audio-record
//! channel, the audio-input row.
//!
//! [`RackPanel::show`] draws it in the rect it is given ([`rack_rect`]: 20 U at the content area's right) and edits the
//! channel in place. What the app must do about it comes back as [`RackActions`]: the param edits to send to the
//! engine, whether the engine needs the song again, where the Modules browser should insert, the modules copied, the
//! preset picker to open and a close request.
//! The geometry ([`flow`], [`content_rect`], [`ui_height`], [`level_popup_rect`]) and the effect on the rack of the
//! card menu ([`apply`]), of `Clear Rack` ([`clear`]), of a browser pick ([`insert`]) or `Paste` ([`paste`]) and of a
//! rack preset ([`load_rack`]) are pure functions.
//!
//! The clipboard is a list of modules: a card's `Copy` puts its one, and `Copy Rack` puts every module of the rack, the
//! instrument included, top to bottom, with all settings, with no message. The Modules picker's `Paste` row pastes.
//!
//! Instrument cards are effect cards with the instrument's height, icon 2 and the `Set Range` menu row. Their panel
//! (tab strip included) is the panel tree's business: the card only gives it the module area ([`content_rect`]).
//!
//! The card's preset button shows its module's preset name and opens the preset picker, as the rack menu's `Load Rack
//! preset` does; its ▶ is drawn only, with no next-preset step.
//!
//! The cards' bound controls (module params, the enable toggle, the common params of the dialog) report their press and
//! their parameter menu's `Add automation track` as a [`Touch`], for the app's recent control. A card's `Delete` on a
//! module that automation lanes target leaves the module in place and asks the app to ask `Delete automation?`
//! ([`RackActions::ask_delete`]); so does `Clear Rack` ([`RackActions::ask_remove`]).
//!
//! A card's `Set Range` (`Set First Note` on a Slicer) learns the module's `ADD1` key range from the next keys played
//! ([`RackPanel::key_played`]); the SENDS section edits the channel's `SEND`s by the channels the host names
//! ([`RackPanel::targets`], [`send_targets`]); `Create FX Channel` asks the host ([`create_send`]).

use std::collections::HashMap;

use eframe::egui::{
    Align2, Area, Color32, Event, FontId, Id, Order, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use honeydew_audio::MeterSnapshot;
use honeydew_audio::device::{AudioCommand, ModuleKey, ModuleMeter};
use honeydew_audio::instrument::KEY_LEARN;

use crate::audio_engine::settings;
use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::midi::links;
use crate::panels::inst::soundfont_player::{self, Press, SoundFonts};
use crate::panels::inst::{slicer, three_osc};
use crate::panels::{self, PanelState, cstm};
use crate::params::{COMMON_PARAMS, ParamSpec, Role, module_spec};
use crate::project_files::PresetSource;
use crate::song::{
    AutomationTarget, Channel, ChannelKind, LaneKind, ModuleInstance, Rack, SendState, Song,
    SynthKind, output_bus,
};
use crate::theme::channel_color;
use crate::ui::buttons::outline_circle_button;
use crate::ui::ctrl_menu::{CtrlAction, Touch};
use crate::ui::gesture;
use crate::ui::layout::{self, Node, Size};
use crate::ui::modules_browser::{InsertAt, add_module, paste_module};
use crate::ui::popup::{self, PopupItem};
use crate::ui::scroller::{self, Axis, Scroller, Y, Zones};
use crate::ui::widgets::{self, Button, Control, ParamMenu};
use crate::ui::{UiCtx, focus_lost};

#[cfg(test)]
mod tests;

// Sizes in U unless noted.
/// The band of the header bar at the top: the bar and the 2 px line under it.
const HEADER_BAR: f32 = 0.8;
/// The gutter on each side of the card column; the left one holds the rack ≡.
const GUTTER: f32 = 1.5;
/// A card's header row; a minimized card is only that.
const CARD_HEADER: f32 = 1.5;
/// Device pixels between cards, and above the first one.
const CARD_GAP_PX: f32 = 5.0;
const FIRST_GAP_PX: f32 = 2.0;
/// `Add Module`, the SENDS header, a send row and `Add Send`.
const ROW: f32 = 1.75;
/// Above the SENDS header.
const SENDS_GAP: f32 = 0.75;
/// How much lower `Add Module` sits when there are no cards and no input row.
const EMPTY_DROP: f32 = 1.0;
/// The audio-input row above the cards, and its record button.
const INPUT_ROW: f32 = 4.0;
const INPUT_BUTTON: f32 = 2.0;
/// The Multiband Compressor's type id: its soloed band goes to the engine without a `CSTM`.
const MULTIBAND: u16 = 36;
/// The Note Echo's type id: its card shows whether echoes are pending.
const NOTE_ECHO: u16 = 47;
/// An icon button's disc: 0.7 × half the rect height.
const ICON_DISC: f32 = 0.7;
/// The rack ≡: an outline circle of 0.7 × half the 1.5 U column, and its icon's size factor.
const RACK_MENU_D: f32 = 1.05;
const RACK_MENU_ICON: f32 = 0.5;
/// Icon size factors in the card header and the send row.
const HEADER_ICON: f32 = 0.6;
const SEND_ICON: f32 = 0.75;
/// The Mix / Post Gain / Wet Pan dialog: its size and content inset.
const DIALOG: Vec2 = vec2(10.0, 8.0);
const DIALOG_INSET: f32 = 0.25;
/// A send level's default: unity.
const SEND_UNITY: f32 = 0.85312;

/// The common params in the dialog's order, with the reserved indices the engine takes.
const COMMON: [(i32, ParamSpec); 3] = [
    (-3, COMMON_PARAMS[2]),
    (-2, COMMON_PARAMS[1]),
    (-1, COMMON_PARAMS[0]),
];

/// The card height of module type `type_id` in U. `None` for an unknown type.
pub fn ui_height(type_id: u16) -> Option<f32> {
    Some(match type_id {
        // Instruments.
        0 => 8.5,
        1 => 7.5,
        2 => 9.5,
        19 => 7.0,
        23 => 9.0,
        33 => 8.5,
        39 => 10.0,
        52 => 8.5,
        // Effects and note effects.
        3 | 7 | 26 | 30 | 51 => 10.0,
        4 | 35 | 36 | 43 | 46 => 9.0,
        5 | 9 | 28 | 48 | 49 => 6.0,
        6 | 10 | 11 | 15 | 18 => 5.5,
        8 => 7.5,
        12 | 16 | 24 | 32 | 34 | 40 => 7.0,
        13 | 14 | 25 | 27 | 44 | 45 => 8.0,
        20 => 5.0,
        29 | 50 => 6.5,
        41 | 47 => 11.0,
        42 => 8.4,
        _ => return None,
    })
}

/// The name a card shows: the registry's display name, or the instrument's. Empty for an unknown type.
pub fn module_name(type_id: u16) -> &'static str {
    module_spec(type_id)
        .map(|s| s.display_name)
        .or_else(|| instrument(type_id).map(SynthKind::label))
        .unwrap_or("")
}

/// The instrument of module type `type_id`, `None` for an effect or an unknown type.
pub(crate) fn instrument(type_id: u16) -> Option<SynthKind> {
    SynthKind::ALL.into_iter().find(|s| s.type_id() == type_id)
}

/// The card icon. Instruments have icon 2 (keys), effects icon 8 (faders) and note effects icon 75 (notes).
fn module_icon(type_id: u16) -> IconId {
    match module_spec(type_id).map(|s| s.role) {
        Some(Role::Effect) => IconId::Mixer,
        Some(Role::NoteEffect) => IconId::Music,
        Some(Role::Instrument) | None => IconId::Piano,
    }
}

/// A card's height in U: its UI height, or the header alone when it is minimized or its type is unknown.
pub fn card_height(module: &ModuleInstance) -> f32 {
    match ui_height(module.type_id) {
        Some(h) if !module.minimized => h,
        _ => CARD_HEADER,
    }
}

/// The module panel's content rect in card `card` of a module `ui_height` U tall: 2 px in from the sides, 1 px under
/// the header row, (card width − 4 px) × ((`ui_height` − 2) U − 1 px), unrounded. Device pixels, like `u`.
pub fn content_rect(card: Rect, ui_height: f32, u: f32) -> Rect {
    Rect::from_min_size(
        card.min + vec2(2.0, CARD_HEADER * u + 1.0),
        vec2(card.width() - 4.0, (ui_height - 2.0) * u - 1.0),
    )
}

/// Where the rack's scrolling content goes, in device pixels. Every row is in the card column.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    /// The audio-input row above the first card, on an audio-record channel.
    pub input: Option<Rect>,
    pub cards: Vec<Rect>,
    pub add_module: Rect,
    /// The SENDS section, when the rack shows sends.
    pub sends: Option<SendsFlow>,
    /// Where the content ends, unrounded.
    pub bottom: f32,
}

/// What the SENDS section holds: a row per send, and whether the `Disconnected from MASTER` row shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SendRows {
    pub rows: usize,
    pub disconnected: bool,
}

/// The SENDS section's rows, in device pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct SendsFlow {
    pub header: Rect,
    /// One per send, in the channel's `SEND` order.
    pub rows: Vec<Rect>,
    /// `Add Send`.
    pub add: Rect,
    /// The `Disconnected from MASTER` label's row.
    pub disconnected: Option<Rect>,
}

/// [`flow_sends`] with `sends` one send row (a channel's default send to the master).
pub fn flow(rack: Rect, scroll: f32, u: f32, input: bool, heights: &[f32], sends: bool) -> Flow {
    let rows = SendRows {
        rows: 1,
        disconnected: false,
    };
    flow_sends(rack, scroll, u, input, heights, sends.then_some(rows))
}

/// Lays out the content of `rack` scrolled up by `scroll`, with cards `heights` U tall, at `u` per U (all in device
/// pixels): the column is 1.5 U in from both sides; with `input`, the 4 U audio-input row starts 2 px under the
/// 0.8 U header bar and the first card directly under it, else the first card starts there; the others 5 px apart;
/// `Add Module` follows like another card (1 U lower when there are no cards and no input row); with `sends`, the
/// SENDS header 0.75 U under it, a row per send, 1 px, `Add Send`, and the `Disconnected from MASTER` row when that
/// applies. Edges are floored to whole pixels one by one.
pub fn flow_sends(
    rack: Rect,
    scroll: f32,
    u: f32,
    input: bool,
    heights: &[f32],
    sends: Option<SendRows>,
) -> Flow {
    let x = (rack.left() + GUTTER * u).floor()..=(rack.right() - GUTTER * u).floor();
    let row = |y: &mut f32, h: f32, gap: f32| {
        let r = Rect::from_x_y_ranges(x.clone(), y.floor()..=(*y + h).floor());
        *y += h + gap;
        r
    };
    let mut y = rack.top() - scroll + HEADER_BAR * u + FIRST_GAP_PX;
    let input = input.then(|| row(&mut y, INPUT_ROW * u, 0.0));
    let cards: Vec<Rect> = heights
        .iter()
        .map(|&h| row(&mut y, h * u, CARD_GAP_PX))
        .collect();
    if cards.is_empty() && input.is_none() {
        y += EMPTY_DROP * u;
    }
    let add_module = row(&mut y, ROW * u, 0.0);
    let sends = sends.map(|sends| {
        y += SENDS_GAP * u;
        let header = row(&mut y, ROW * u, 0.0);
        let rows = (0..sends.rows).map(|_| row(&mut y, ROW * u, 0.0)).collect();
        y += 1.0;
        SendsFlow {
            header,
            rows,
            add: row(&mut y, ROW * u, 0.0),
            disconnected: sends.disconnected.then(|| row(&mut y, ROW * u, 0.0)),
        }
    });
    Flow {
        input,
        cards,
        add_module,
        sends,
        bottom: y,
    }
}

/// The rack's width, in U, capped at the area's.
const RACK_W: f32 = 20.0;

/// The rack over the content `area` (the screen above the keyboard panel), slid in by `open` (0 closed … 1 open): R =
/// trunc(min(20 U, W)) wide and the area's height, its left edge at the area's right − `open`·R. Closed, it lies just
/// past the area's right edge.
pub fn rack_rect(area: Rect, open: f32, m: &Metrics) -> Rect {
    let ppp = m.ppp();
    // Both are whole pixels: rounding the width first keeps float noise from truncating a pixel away.
    let w = (RACK_W * m.u_px()).min((area.width() * ppp).round()) / ppp;
    let left = area.right() - open * w;
    Rect::from_x_y_ranges(left..=left + w, area.y_range())
}

/// One frame of the rack's slide: `r` moves half way to `target`, snapping to it once within a pixel of the area's
/// `width_px`.
pub fn slide(r: f32, target: f32, width_px: f32) -> f32 {
    let r = r + 0.5 * (target - r);
    if (target - r).abs() < 1.0 / width_px {
        target
    } else {
        r
    }
}

/// The channel-colour divider of the rack `rack` (in px): 2 px wide at its left edge + 1, its full height.
pub fn divider(rack: Rect) -> Rect {
    Rect::from_x_y_ranges(rack.left() + 1.0..=rack.left() + 3.0, rack.y_range())
}

/// The `▶` notch on the divider of the rack `rack` (in px) at a U of `u` px: half-height s = trunc(0.5 U), its base at
/// x = trunc(left + 3) from y_c − s to y_c + s, its apex at (x + s, y_c), y_c = trunc(top + (height − 2s)/2).
pub fn notch(rack: Rect, u: f32) -> [Pos2; 3] {
    let s = (0.5 * u).trunc();
    let x = (rack.left() + 3.0).trunc();
    let y = (rack.top() + (rack.height() - 2.0 * s) / 2.0).trunc();
    [pos2(x, y - s), pos2(x + s, y), pos2(x, y + s)]
}

/// Where a held card whose on-screen top is `top` goes with the finger at height `finger_y` (`u` per U and `gap`, the
/// 5 px between cards, in the same unit). Held, every card is 1.5 U tall. Nowhere while the finger is in its band
/// [top − gap, top + 1.5 U + gap], at any x; else a slot up (the card menu's `Move up`) while the finger is at most
/// 1.5 U below the top, and a slot down below that.
pub fn drag_step(top: f32, finger_y: f32, u: f32, gap: f32) -> Option<CardAction> {
    let bottom = top + CARD_HEADER * u;
    if (top - gap..=bottom + gap).contains(&finger_y) {
        None
    } else if finger_y <= bottom {
        Some(CardAction::MoveUp)
    } else {
        Some(CardAction::MoveDown)
    }
}

/// How long (s) after a step a held card does not step again, its new top at `top` and the finger at height
/// `finger_y` (`u` per U, same unit): 0.3·(1.15 − 0.8·min(1, |finger y − top| / 1.5 U)), 345 ms on the top and 105 ms
/// 1.5 U from it or more.
pub fn step_interval(top: f32, finger_y: f32, u: f32) -> f64 {
    let far = ((finger_y - top).abs() / (CARD_HEADER * u)).min(1.0);
    0.3 * (1.15 - 0.8 * f64::from(far))
}

/// The items of a card's ≡ menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// Minimize, or Maximize when minimized.
    Minimize,
    SavePreset,
    MoveUp,
    MoveDown,
    Copy,
    InsertAbove,
    InsertBelow,
    Replace,
    Delete,
    /// `Set Range` (`Set First Note` on a Slicer): instrument cards only.
    SetRange,
    /// `Set Full Range`: instrument cards but the Slicer's, while the key range is not 0–127.
    SetFullRange,
}

/// What a preset picker loads into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetTarget {
    /// A card's preset button: a module preset into the module with this instance id.
    Module(i32),
    /// The rack ≡ `Load Rack preset`: a rack preset replaces the rack's modules ([`load_rack`]).
    RackReplace,
    /// The Modules browser's `Rack Preset`: a rack preset's modules go in where its `Paste` puts modules ([`paste`]).
    RackInsert(InsertAt),
    /// The add-channel `Rack`: a new Rack channel holding a rack preset, last or (`Insert Channel`) at this index.
    NewChannel(Option<usize>),
    /// `Load Drumset` (the rack ≡ menu's and the drums track menu's): a drum kit template replaces the channel's kit.
    Drumset,
    /// A chord set (`.chords`) replaces the song's `KEYB` set. It belongs to the song, not a channel. The keyboard
    /// settings' ChordSet ≡ `Load` opens it (`views/keyboard_settings.rs`).
    ChordSet,
    /// The Instruments picker, opened by the add-channel `Instrument`: an instrument preset into the rack's top card
    /// when it is of the preset's type, else in place of it, appended at the bottom.
    Instruments,
}

/// What a card action did to the rack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// The chain changed: the engine needs the song again.
    ChainChanged,
    /// The Modules browser should open to insert there.
    Browse(InsertAt),
    /// The card was minimized or maximized: a song edit the audio does not hear.
    Minimized,
    /// `Copy`: the module goes on the app clipboard, alone; the rack is unchanged.
    Copied,
    /// `Save preset`: the preset dialog opens on the module; the rack is unchanged.
    SavePreset,
}

/// Applies `action` from the menu of module `instance_id`'s card to `rack`. Moves past either end and a missing module
/// do nothing, and so does `Set Range` here: its key-learn is the panel's ([`RackPanel::key_played`]).
pub fn apply(rack: &mut Rack, instance_id: i32, action: CardAction) -> Outcome {
    let Some(index) = rack.index_of(instance_id) else {
        return Outcome::Nothing;
    };
    let chain = |changed| {
        if changed {
            Outcome::ChainChanged
        } else {
            Outcome::Nothing
        }
    };
    match action {
        CardAction::Minimize => {
            rack.modules[index].minimized ^= true;
            Outcome::Minimized
        }
        CardAction::MoveUp => chain(rack.move_up(instance_id)),
        CardAction::MoveDown => chain(rack.move_down(instance_id)),
        CardAction::Delete => chain(rack.remove(instance_id).is_some()),
        CardAction::InsertAbove => Outcome::Browse(InsertAt::Above(instance_id)),
        CardAction::InsertBelow => Outcome::Browse(InsertAt::Below(instance_id)),
        CardAction::Replace => Outcome::Browse(InsertAt::Replace(instance_id)),
        CardAction::Copy => Outcome::Copied,
        CardAction::SavePreset => Outcome::SavePreset,
        CardAction::SetRange => Outcome::Nothing,
        // The engine reads the key range at build.
        CardAction::SetFullRange => {
            let file = &mut rack.modules[index].file;
            (file.key_low, file.key_high) = (0, 127);
            Outcome::ChainChanged
        }
    }
}

/// `Clear Rack`: removes every module but the instruments.
pub fn clear(rack: &mut Rack) -> Outcome {
    let before = rack.modules.len();
    rack.modules.retain(|m| instrument(m.type_id).is_some());
    if rack.modules.len() < before {
        Outcome::ChainChanged
    } else {
        Outcome::Nothing
    }
}

/// An edit [`hold_removal`] held back, for the app to ask `Delete automation?` and make it on the answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    /// The channel's index.
    pub channel: usize,
    /// The channel as the edit leaves it.
    pub after: Channel,
    /// The modules a rack preset's load skipped: its `Skipped N incompatible module(s)` shows once the edit is made.
    pub skipped: usize,
}

/// Puts `channel` back as it was (`before`) when an edit removed modules that its automation lanes target, and
/// returns the edited channel: the app asks `Delete automation?` first. `None` leaves the edit.
pub fn hold_removal(channel: &mut Channel, before: Channel) -> Option<Channel> {
    let gone = |id: i32| before.rack.get(id).is_some() && channel.rack.get(id).is_none();
    let automated = (before.lanes.iter()).any(|l| l.target.is_some_and(|t| gone(t.instance_id)));
    automated.then(|| std::mem::replace(channel, before))
}

/// Adds module `type_id`, picked in the Modules browser, at `at` (what [`RackActions::open_browser`] asked for), with
/// its defaults and a fresh instance id. The chain always changes: the engine needs the song again, or it drops the
/// new module's param edits.
pub fn insert(rack: &mut Rack, type_id: u16, at: InsertAt) -> Outcome {
    add_module(rack, type_id, at);
    Outcome::ChainChanged
}

/// Loads the rack preset `modules`, in file order, into `channel`'s rack, each with a fresh instance id.
/// - Without `at` (`Load Rack preset`, a new Rack channel) they replace the rack's modules, and the preset's
///   instruments the channel's, which stay when the preset has none. Each goes where `Add Module` puts it; an
///   instrument goes under the note effects and the instruments already there, above any effect.
/// - With `at` (the Modules browser's `Rack Preset`) they go where [`paste`] puts the same modules: from `+` appended
///   at the bottom, else at `at`, in file order.
///
/// A module is skipped by the rule `Paste` follows ([`attaches`]): an id above 52, and an instrument where the channel
/// does not take one ([`takes_instruments`]: Master, Effect Track, audio, Drums), as the browser's SYNTH rows do; a
/// channel that does takes all the instruments, beside the ones it has. Returns whether the chain changed (never when
/// every module is skipped, or there are none) and how many modules were skipped. [`load_rack_into`] also names the
/// channel after the instrument taken.
pub(crate) fn load_rack(
    channel: &mut Channel,
    modules: &[ModuleInstance],
    at: Option<InsertAt>,
) -> (Outcome, usize) {
    let room = takes_instruments(channel);
    let skipped = modules.iter().filter(|m| !attaches(m, room)).count();
    if let Some(at) = at {
        return (paste(channel, modules, at), skipped);
    }
    if skipped == modules.len() {
        return (Outcome::Nothing, skipped);
    }
    let is_instrument = |m: &ModuleInstance| instrument(m.type_id).is_some();
    let takes = room && modules.iter().any(is_instrument);
    let rack = &mut channel.rack;
    rack.modules.retain(|m| is_instrument(m) && !takes);
    let note =
        |m: &ModuleInstance| module_spec(m.type_id).is_some_and(|s| s.role == Role::NoteEffect);
    for module in modules.iter().filter(|m| attaches(m, room)) {
        let here = if is_instrument(module) {
            // Under the instruments there, else under the note effects, above any effect.
            let last = rack.modules.iter().rfind(|m| is_instrument(m));
            let first = rack.modules.iter().find(|m| !note(m));
            match (last, first) {
                (Some(m), _) => InsertAt::Below(m.instance_id),
                (None, Some(m)) => InsertAt::Above(m.instance_id),
                (None, None) => InsertAt::End,
            }
        } else {
            InsertAt::End
        };
        paste_module(rack, module, here);
    }
    (Outcome::ChainChanged, skipped)
}

/// [`load_rack`] into channel `index` of `song`; the instrument it takes, if any, names the channel
/// ([`name_song_channel`]): its preset, or none while it is at its defaults (`Default`).
pub fn load_rack_into(
    song: &mut Song,
    index: usize,
    modules: &[ModuleInstance],
    at: Option<InsertAt>,
) -> (Outcome, usize) {
    let Some(ch) = song.channels.get_mut(index) else {
        return (Outcome::Nothing, 0);
    };
    let last = ch.rack.last_instance_id;
    let loaded = load_rack(ch, modules, at);
    let taken =
        (ch.rack.modules.iter()).find(|m| m.instance_id > last && instrument(m.type_id).is_some());
    if let Some(module) = taken {
        let preset = match module.file.preset_name.as_str() {
            "Default" => String::new(),
            name => name.to_owned(),
        };
        name_song_channel(song, index, preset);
    }
    loaded
}

/// [`name_channel`] for channel `index` of `song`, whose other channels' names are taken.
pub fn name_song_channel(song: &mut Song, index: usize, preset: String) {
    if index >= song.channels.len() {
        return;
    }
    let (before, rest) = song.channels.split_at_mut(index);
    let (ch, after) = rest.split_first_mut().expect("index is in range");
    let taken = |name: &str| before.iter().chain(after.iter()).any(|c| c.name == name);
    name_channel(ch, preset, &taken);
}

/// An instrument's preset (`preset`, "" for `Default`) is the channel's (in its `CHHD`), and so is its
/// name while it is the automatic one: a preset's name takes the place of a name no one typed (two channels may both
/// be named after one preset), and `Default` gives the writer's base name back to a name that came from the preset it
/// replaces, unless another channel has that name (`taken`): new channels are numbered `<label> N`, not by id. The name
/// then stays, and a save writes it as the custom name.
pub fn name_channel(ch: &mut Channel, preset: String, taken: &dyn Fn(&str) -> bool) {
    let automatic = match preset.as_str() {
        "" => named(ch, &ch.file.preset_name),
        _ => {
            let read = ch.file.names.0.as_ref().map(|(base, _)| base.as_str());
            let autos = [ch.file.preset_name.as_str(), ch.kind.label()];
            autos.into_iter().chain(read).any(|auto| named(ch, auto))
        }
    };
    let name = match preset.as_str() {
        "" => Some(base_name(ch)).filter(|base| !taken(base)),
        name => Some(name.to_owned()),
    };
    if let Some(name) = name.filter(|_| automatic) {
        ch.name = name;
    }
    ch.file.preset_name = preset;
}

/// The Instruments picker's channel name: the preset's name, or `<instrument> <channel id>` for an empty or `Default`
/// one, while the channel's name is an automatic one: its kind's label (`Instrument`, so the first pick renames it),
/// the last preset's, the base name read, or a name this rule gave. The preset's name is the channel's
/// ([`name_channel`]).
pub fn name_instrument_channel(song: &mut Song, index: usize, preset: &str, instrument: &str) {
    let Some(ch) = song.channels.get_mut(index) else {
        return;
    };
    let preset = match preset {
        "Default" => "",
        name => name,
    };
    let given: Vec<String> = (SynthKind::ALL.iter())
        .map(|k| format!("{} {}", k.label(), ch.id))
        .collect();
    let read = ch.file.names.0.as_ref().map(|(base, _)| base.as_str());
    let autos = [ch.kind.label(), ch.file.preset_name.as_str()].into_iter();
    let automatic =
        (autos.chain(read).chain(given.iter().map(String::as_str))).any(|a| named(ch, a));
    if automatic {
        ch.name = match preset {
            "" => format!("{instrument} {}", ch.id),
            name => name.to_owned(),
        };
    }
    ch.file.preset_name = preset.to_owned();
}

/// The base name a save writes (`flm::write`, `CHHD` vstr32 #1): the one read, else `<Kind> <id>`.
fn base_name(ch: &Channel) -> String {
    match &ch.file.names.0 {
        Some((base, _)) => base.clone(),
        None => format!("{} {}", ch.kind.base_label(), ch.id),
    }
}

/// Whether `ch`'s name is `auto`, or `auto` numbered as a new or cloned channel's is (`Synth 2`), and not a custom name
/// the file says the user typed (the `CHHD` custom name, set only on a rename).
// ponytail: nothing records a rename made in this app, so a typed name that matches `auto` counts as automatic.
fn named(ch: &Channel, auto: &str) -> bool {
    let name = ch.name.as_str();
    let unnumbered = (name.rsplit_once(' '))
        .filter(|(_, n)| n.parse::<u32>().is_ok())
        .map_or(name, |(head, _)| head);
    let typed =
        (ch.file.names.0.as_ref()).is_some_and(|(_, custom)| !custom.is_empty() && custom == name);
    !typed && !auto.is_empty() && (auto == name || auto == unnumbered)
}

/// The Modules browser's `Paste` of `clipboard` into `channel`'s rack at `at`: every compatible clipboard module goes
/// in, in clipboard order, as a new module with a fresh id and its copied settings: from the rack's add-module `+`,
/// appended at the bottom; from `Insert above` / `Insert below`, at that position; from `Replace`, in place of that
/// module, which is deleted. Instrument modules go only into a channel whose primary lane is a notes lane (not drum and
/// audio channels); module ids above 52 are skipped.
///
/// Each module after the first goes below the one before, and the bottom is the bottom for a note effect too (a pick
/// goes up to the note effects instead). With nothing compatible it returns [`Outcome::Nothing`] and the rack is as it
/// was, a `Replace`'s module included: the caller flashes `Cannot paste module`.
pub fn paste(channel: &mut Channel, clipboard: &[ModuleInstance], at: InsertAt) -> Outcome {
    let notes = takes_instruments(channel);
    let rack = &mut channel.rack;
    let mut at = match (at, rack.modules.last()) {
        (InsertAt::End, Some(last)) => InsertAt::Below(last.instance_id),
        _ => at,
    };
    let mut outcome = Outcome::Nothing;
    for module in clipboard {
        if !attaches(module, notes) {
            continue;
        }
        at = InsertAt::Below(paste_module(rack, module, at));
        outcome = Outcome::ChainChanged;
    }
    outcome
}

/// Whether `channel` takes instrument modules: only a channel whose primary lane is a notes lane does (not drum and
/// audio channels), with no limit on how many a rack holds. [`paste`] follows it, and so do the Modules browser's SYNTH
/// rows and a rack preset's load ([`load_rack`]).
pub fn takes_instruments(channel: &Channel) -> bool {
    (channel.lanes.first()).is_some_and(|l| l.kind == LaneKind::Notes)
}

/// Whether a channel takes `module`, the one routine `Paste` and a rack preset's load share: module ids above 52 are
/// skipped, and an instrument goes only where `notes` (the channel [takes instruments](takes_instruments)).
fn attaches(module: &ModuleInstance, notes: bool) -> bool {
    module.type_id <= 52 && (notes || instrument(module.type_id).is_none())
}

/// The ≡ menu of a card of module type `type_id`, two columns, every item enabled. An instrument's adds `Set Range`
/// (`Set First Note` on a Slicer) at the foot of the first column, and under it `Set Full Range` while its key range is
/// not `full` (never on a Slicer).
fn card_menu(minimized: bool, type_id: u16, full: bool) -> Vec<PopupItem<CardAction>> {
    use CardAction::*;
    let minimize = if minimized { "Maximize" } else { "Minimize" };
    let (set_range, full_range) = match instrument(type_id) {
        Some(SynthKind::Slicer) => (Some((SetRange, 0, 4, "Set First Note")), None),
        Some(_) => (
            Some((SetRange, 0, 4, "Set Range")),
            (!full).then_some((SetFullRange, 0, 5, "Set Full Range")),
        ),
        None => (None, None),
    };
    [
        (Minimize, 0, 0, minimize),
        (SavePreset, 0, 1, "Save preset"),
        (MoveUp, 0, 2, "Move up"),
        (MoveDown, 0, 3, "Move down"),
        (Copy, 1, 0, "Copy"),
        (InsertAbove, 1, 1, "Insert above"),
        (InsertBelow, 1, 2, "Insert below"),
        (Replace, 1, 3, "Replace"),
        (Delete, 1, 4, "Delete"),
    ]
    .into_iter()
    .chain(set_range)
    .chain(full_range)
    .map(|(action, col, row, label)| PopupItem::new(action, col, row, label))
    .collect()
}

/// Whether `module`'s key range is the full 0–127.
fn full_range(module: &ModuleInstance) -> bool {
    (module.file.key_low, module.file.key_high) == (0, 127)
}

/// The key-learn of module `instance_id` on channel `channel` turned on or off, for the engine.
fn learn(channel: usize, instance_id: i32, on: bool) -> SettingChange {
    SettingChange {
        channel,
        instance_id,
        key: KEY_LEARN,
        value: if on { 1.0 } else { 0.0 },
    }
}

/// A channel a send can go to, as the host gives it ([`send_targets`]).
#[derive(Clone, Debug, PartialEq)]
pub struct SendTarget {
    /// Its channel id (`CHHD`@1080), which `SEND`@0 names.
    pub id: i32,
    pub name: String,
    /// `Add Send` offers it: the master or a send channel that sends nothing back to the rack's channel.
    pub offered: bool,
    /// Its send bus (`RHED`@8): ≥ 1 on a send channel.
    pub bus: i32,
    /// A path of its sends reaches the master (the master itself does).
    pub to_master: bool,
}

/// What channel `channel`'s sends can name: every other channel of `song`, in song order. `Add Send` offers each send
/// channel by its name, and the master too (it can come back after its send was removed, and the menu has no other
/// way); a channel whose sends lead back to `channel` is not offered, so the app never makes a cycle.
pub fn send_targets(song: &Song, channel: usize) -> Vec<SendTarget> {
    let Some(this) = song.channels.get(channel) else {
        return Vec::new();
    };
    // Whether a path of sends leads from channel id `from` to `to`.
    let reaches = |from: i32, to: i32| {
        let (mut seen, mut stack) = (Vec::new(), vec![from]);
        while let Some(id) = stack.pop() {
            if id == to {
                return true;
            }
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let c = song.channels.iter().find(|c| c.id == id);
            stack.extend(
                c.into_iter()
                    .flat_map(|c| &c.file.sends)
                    .map(|s| s.target_channel_id),
            );
        }
        false
    };
    (song.channels.iter().enumerate())
        .filter(|&(index, _)| index != channel)
        .map(|(index, c)| SendTarget {
            id: c.id,
            name: c.name.clone(),
            offered: (index == 0 || c.kind == ChannelKind::EffectTrack) && !reaches(c.id, this.id),
            bus: c.bus_id,
            to_master: reaches(c.id, 0),
        })
        .collect()
}

/// Whether `channel` no longer routes to the master: none of its sends goes there, directly or through `targets`.
pub fn disconnected(channel: &Channel, targets: &[SendTarget]) -> bool {
    !(channel.file.sends.iter()).any(|s| {
        s.target_channel_id == 0
            || (targets.iter()).any(|t| t.id == s.target_channel_id && t.to_master)
    })
}

/// `Create FX Channel` from channel `channel`'s rack ([`RackActions::create_send`]): a new send channel at the end of
/// `song`, and channel `channel`'s send `at` goes to it (`at` past the last send: a new send, at the defaults). Returns
/// the new channel's index; `None` once the channel ids are used up, or for no such channel.
pub fn create_send(song: &mut Song, channel: usize, at: usize) -> Option<usize> {
    if channel >= song.channels.len() {
        return None;
    }
    let new = song.insert_channel(song.channels.len(), ChannelKind::EffectTrack)?;
    let id = song.channels[new].id;
    let buses = song.send_buses();
    let ch = &mut song.channels[channel];
    match ch.file.sends.get_mut(at) {
        Some(send) => send.target_channel_id = id,
        None => ch.file.sends.push(SendState {
            target_channel_id: id,
            ..SendState::default()
        }),
    }
    ch.output_bus = output_bus(&ch.file.sends, &buses);
    Some(new)
}

/// An item of the send-target menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendPick {
    Target(i32),
    Create,
}

/// The items of the rack ≡ menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RackItem {
    LoadDrumset,
    LoadPreset,
    SavePreset,
    CopyRack,
    ClearRack,
    MinimizeAll,
    MaximizeAll,
}

/// The rack ≡ menu, one column. `Load Drumset` is only there for a drums channel. `Save Rack preset` is disabled
/// while the rack is `empty`: a preset of nothing. `Copy Rack` copies an empty rack too: nothing, which `Paste` then
/// flashes.
fn rack_menu(drums: bool, empty: bool) -> Vec<PopupItem<RackItem>> {
    use RackItem::*;
    [
        (LoadDrumset, "Load Drumset"),
        (LoadPreset, "Load Rack preset"),
        (SavePreset, "Save Rack preset"),
        (CopyRack, "Copy Rack"),
        (ClearRack, "Clear Rack"),
        (MinimizeAll, "Minimize all"),
        (MaximizeAll, "Maximize all"),
    ]
    .into_iter()
    .filter(|&(item, _)| drums || item != LoadDrumset)
    .enumerate()
    .map(|(row, (item, label))| PopupItem {
        disabled: item == SavePreset && empty,
        ..PopupItem::new(item, 0, row, label)
    })
    .collect()
}

/// A param edit, already written to the song: send it to the engine as [`ParamChange::command`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamChange {
    /// The channel's index in the song.
    pub channel: usize,
    pub instance_id: i32,
    /// The module param (0 is Enable), or −1, −2, −3 for Post Gain, Mix and Wet Pan.
    pub index: i32,
    pub value: f32,
}

impl ParamChange {
    pub fn command<R>(self) -> AudioCommand<R> {
        AudioCommand::SetParam {
            channel: self.channel,
            instance_id: self.instance_id,
            index: self.index,
            value: self.value,
        }
    }
}

/// A module setting edit (a control without a param, by its panel `#n`): send it to the engine as
/// [`SettingChange::command`], so the setting sounds without a rebuild (engine-notes item 12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettingChange {
    /// The channel's index in the song.
    pub channel: usize,
    pub instance_id: i32,
    pub key: u32,
    pub value: f32,
}

impl SettingChange {
    pub fn command<R>(self) -> AudioCommand<R> {
        AudioCommand::SetSetting {
            channel: self.channel,
            instance_id: self.instance_id,
            key: self.key,
            value: self.value,
        }
    }
}

/// What the app must do after a pass of [`RackPanel::show`]. egui may run a frame in several passes: act on every
/// pass's actions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RackActions {
    /// Param edits of this pass, in order.
    pub params: Vec<ParamChange>,
    /// The rack's chain changed (a module moved or was deleted, the rack cleared), or a note-effect edit or an edit of
    /// a setting the engine reads at build ended (note-effect events, the DW Sampler's zones and the Slicer's slices
    /// are prepared ahead of playback): build a new engine from the song and send `ReplaceSong`.
    pub replace_song: bool,
    /// Cards were minimized or maximized: the song changed, with nothing for the audio.
    pub minimized: bool,
    /// A card's settings changed its module's `CSTM`: the song changed. The audio gets the edits as
    /// [`Self::setting_changes`].
    pub settings: bool,
    /// Setting edits of this pass (the panel's hook's included), each module's in key order.
    pub setting_changes: Vec<SettingChange>,
    /// Open the Modules browser to insert a module there. Adding the picked module changes the chain: the engine
    /// needs the song again then too.
    pub open_browser: Option<InsertAt>,
    /// A card's `Copy` (its module) or the rack's `Copy Rack` (every module, top to bottom): put these on the app
    /// clipboard in place of what it held, for the browser's `Paste`. Not a song edit.
    pub copy: Option<Vec<ModuleInstance>>,
    /// A card's `Save preset` or the rack's `Save Rack preset`: open the preset dialog on this. Not a song edit.
    pub preset: Option<PresetSource>,
    /// A card's preset button or the rack's `Load Rack preset`: open the preset picker to load into this.
    pub picker: Option<PresetTarget>,
    /// A SoundFont Player's `Patch` spinner with no patches to list (or the tap on its error): open the SoundFont picker
    /// for the module with this instance id.
    pub soundfont: Option<i32>,
    /// `Custom` picked in a 3x Osc oscillator's Shape menu (the Shape edit itself is in [`Self::params`]), or a
    /// Slicer's `LOAD SAMPLE` or a tap on its waveform: open the sample picker for the module with this instance id.
    pub module_sample: Option<i32>,
    /// The Slicers whose Sensitivity changed, as [`Self::replace_song`] comes: a slice re-detect, which rewrites the
    /// current clip while the module's AUTO flag is on.
    pub redetect: Vec<i32>,
    /// A Slicer's momentary `AUTO` was pressed: rewrite the current clip with its slices now.
    pub auto_clip: Option<i32>,
    /// The notch was clicked: close the rack.
    pub close: bool,
    /// The `INPUT` menu opened: list the input devices into [`RackPanel::inputs`] for it.
    pub list_inputs: bool,
    /// An `INPUT` menu pick: the channel's input from now on (kept by the host).
    pub input: Option<Route>,
    /// A bound control was pressed, or its parameter menu picked `Add automation track`: the recent control.
    pub touch: Option<Touch>,
    /// A card's `Delete` on the module with this instance id, which automation lanes target: it is still there, for the
    /// app to ask `Delete automation?` and delete it on the answer.
    pub ask_delete: Option<i32>,
    /// `Clear Rack` removing modules that automation lanes target: the channel as it leaves it, held back
    /// ([`hold_removal`]) for the app to ask `Delete automation?` first.
    pub ask_remove: Option<Channel>,
    /// `Create FX Channel` in the send-target menu: make a new send channel and point the channel's send at this index
    /// to it, past the last one for a new send ([`create_send`]); a final song edit.
    pub create_send: Option<usize>,
}

/// A card's panel state, and its module's `CSTM` blob as the state last matched it: a card is seeded from the blob
/// when it is made and whenever the blob changed under it (an undo or redo
/// that brings its channel back, a pick, a song opened), and its settings edits are stored into it.
#[derive(Clone, Debug)]
struct Card {
    state: PanelState,
    cstm: Option<Vec<u8>>,
}

/// What one card's pass changed ([`RackPanel::card`]).
struct CardEdits {
    /// The params the user changed.
    changed: Vec<i32>,
    /// The module's `CSTM` changed.
    stored: bool,
    /// Its preset button was clicked.
    presets: bool,
    /// What a SoundFont Player's patch selector asked for.
    press: Option<Press>,
    /// A Slicer asked for a loop ([`slicer::asks_for_loop`]).
    sample: bool,
    /// A Slicer's `AUTO` was pressed ([`slicer::auto_pressed`]).
    auto: bool,
    /// The touch of a bound control.
    touch: Option<Touch>,
    /// The settings the engine gets, by key, in key order.
    sent: Vec<(u32, f32)>,
    /// The press on the module-name label: a long one holds the card to reorder it.
    header: gesture::Press,
    /// A double tap on the module-name label.
    double_tap: bool,
}

/// A card held to reorder it.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// Its module's instance id.
    id: i32,
    /// Where the press went down (points): the drag starts once the finger is more than 0.5 U from it vertically.
    origin: Pos2,
    /// The drag has started: each move event runs the step rule.
    started: bool,
    /// When it last stepped, and for how long after that the step rule's evaluations are dropped.
    stepped: Option<(f64, f64)>,
}

/// The rack panel's own state: scrolling, the open menu or dialog, and each card's panel state.
#[derive(Debug, Default)]
pub struct RackPanel {
    /// The channel index last drawn. Instance ids repeat across channels (every instrument is id 2), so another
    /// channel's rack starts afresh: scroll, open menu or dialog. Panel states are kept.
    channel: usize,
    /// How far the content is scrolled up, in points: past either end while a finger pulls it there, a card is held,
    /// or it springs back.
    scroll: f32,
    /// Its scroll controller: the touch, the fling, the spring.
    scroller: Scroller,
    /// Each card's panel state (its settings and tab page), by channel index, instance id and module type, so a
    /// reused id of another type starts afresh.
    panels: HashMap<(usize, i32, u16), Card>,
    /// The open card menu: its card and the point it is centred on.
    card_menu: Option<(i32, Pos2)>,
    /// The open rack menu's centre.
    rack_menu: Option<Pos2>,
    /// The open Mix / Post Gain / Wet Pan dialog: its card and the icon centre it hangs from.
    dialog: Option<(i32, Pos2)>,
    /// The open `INPUT` menu's centre.
    input_menu: Option<Pos2>,
    /// The open SoundFont Player patch menu: its card and the point it is centred on.
    patch_menu: Option<(i32, Pos2)>,
    /// The input devices for the open `INPUT` menu, and the channel's input among them: the host lists them when
    /// [`RackActions::list_inputs`] asks. `None` until then; the menu shows once they are there.
    pub inputs: Option<(Vec<InputDevice>, Option<InputChoice>)>,
    /// The input row's: the `Enable multitrack recording` setting (it shows the arm button), and the 25 ms peaks of the
    /// channel's input, newest first, for the level meter; `None` without an open input. The host sets them each frame.
    pub multitrack: bool,
    pub input_peaks: Option<[f32; METER_RINGS]>,
    /// The `Set middle C to C4` setting: the cards' app octave offset ([`panels::flag::OCTAVE_OFFSET`]). The host sets
    /// it each frame.
    pub middle_c_is_c4: bool,
    /// The frames the input row has been shown since the rack showed this channel ([`input_appear`]), and the last
    /// egui frame (`cumulative_frame_nr`) the rack was shown in: a gap is a rack opened again.
    input_frames: u32,
    shown_frame: u64,
    /// A note effect's param, or a setting the engine reads at build ([`settings::rebuilds`]), changed; once the
    /// pointer is up the engine needs the song again.
    note_edit: bool,
    /// The Slicers whose rebuild setting (Sensitivity) changed since the last [`RackActions::replace_song`].
    redetect: Vec<i32>,
    /// The pointer is over an overlay: the wheel is not the rack's. The host sets it each frame.
    pub covered: bool,
    /// This frame's module meters, the output snapshot's, or `None` without a running stream: each card posts its
    /// module's meter and analysis for the panel's draws ([`panels::post_feed`]). The host sets it each frame.
    pub meters: Option<Vec<ModuleMeter>>,
    /// The patches of the SoundFonts the SoundFont Players name, as read: the host reads each one the first time a
    /// rack shows it ([`soundfont_player::wire`] shows one it has not read yet as loading).
    pub soundfonts: SoundFonts,
    /// The channels the rack's sends can name: the host sets it each frame ([`send_targets`]).
    pub targets: Vec<SendTarget>,
    /// The key-learn of `Set Range` / `Set First Note`: the module's instance id, and the low key once played.
    learning: Option<(i32, Option<u8>)>,
    /// The keys played on the channel since the last pass ([`Self::key_played`]).
    played: Vec<u8>,
    /// The open send-target menu: the send it retargets (`None`: `Add Send`) and its centre.
    send_menu: Option<(Option<usize>, Pos2)>,
    /// The card held to reorder it. Meanwhile every card shows minimized, a view the song does not hold.
    drag: Option<Drag>,
    /// The swap slides by instance id: how far (U) each card is drawn from its laid-out place.
    slides: HashMap<i32, f32>,
    /// The frame the slides last decayed in.
    slid: u64,
}

impl RackPanel {
    /// Follows the channels to `channel(index)` (`None`: gone) after they moved, so that each keeps its panel states
    /// and the rack drawn keeps its scroll and open menu. A gone channel's panel states go, and so does its rack's
    /// state.
    pub fn rekey(&mut self, channel: &dyn Fn(usize) -> Option<usize>) {
        let panels = std::mem::take(&mut self.panels);
        let panels = (panels.into_iter())
            .filter_map(|((c, id, ty), state)| Some(((channel(c)?, id, ty), state)))
            .collect();
        match channel(self.channel) {
            Some(index) => (self.channel, self.panels) = (index, panels),
            None => {
                *self = Self {
                    panels,
                    ..Self::default()
                }
            }
        }
    }

    /// The host calls it for each live key played (keyboard, pads, MIDI), on the song's channel `channel`; a key on
    /// another channel than the rack's is not its. While a card's `Set Range` learns, the next two keys are the low and
    /// high bounds of its module's key range, in either order; a Slicer's `Set First Note` takes one, as its `ADD1` low
    /// key. The next [`Self::show`] makes the edit.
    pub fn key_played(&mut self, channel: usize, key: u8) {
        if self.learning.is_some() && channel == self.channel {
            self.played.push(key.min(127));
        }
    }

    /// Ends a running key-learn with nothing learnt (the song is going): the setting that turns it off, for the
    /// engine and the screen-only settings `AudioLink` re-sends to each new engine.
    pub fn end_learn(&mut self) -> Option<SettingChange> {
        self.played.clear();
        let (id, _) = self.learning.take()?;
        Some(learn(self.channel, id, false))
    }

    /// What the keyboard panel shows while a key-learn waits.
    pub fn learn_prompt(&self) -> Option<&'static str> {
        match self.learning? {
            (_, None) => Some("Play a note to set the lower range bound..."),
            (_, Some(_)) => Some("Play a note to set the upper range bound..."),
        }
    }

    /// Gives channel `to` a copy of channel `from`'s panel states (a cloned channel's rack), replacing its own. A
    /// Multiband's soloed band stays behind: it is screen-only state that only the source's engine holds, so the
    /// clone's card works it out and sends it on its first pass.
    pub fn copy_panels(&mut self, from: usize, to: usize) {
        self.panels.retain(|&(c, ..), _| c != to);
        let copies: Vec<_> = (self.panels.iter())
            .filter(|&(&(c, ..), _)| c == from)
            .map(|(&(_, id, ty), card)| {
                let mut card = card.clone();
                if ty == MULTIBAND {
                    card.state.settings.remove(&cstm::MULTIBAND_SOLO_BAND);
                }
                ((to, id, ty), card)
            })
            .collect();
        self.panels.extend(copies);
    }

    /// Draws the rack of `channel` (the song's channel `index`) in `rect` and returns what the app must do. Edits go
    /// to the channel directly.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        rect: Rect,
        index: usize,
        channel: &mut Channel,
    ) -> RackActions {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let (ppp, u) = (m.ppp(), m.u_px());
        let hue = channel_color(channel.hue, cx.theme);
        let ink = hue.to_color32();
        // The theme colour that stands out most against the channel colour.
        let contrast = colors
            .best_contrasting(hue, &colors.contrast_candidates())
            .to_color32();
        let px = rect * ppp;
        let rack = Rect::from_min_max(px.min.round(), px.max.round());
        let rect = rack / ppp;
        let mut actions = RackActions::default();
        // The links of the channel's `CCLK`: its bound controls' `Unlink from MIDI CC`.
        panels::set_cc_links(ui.ctx(), links::links(&channel.file.cc_links));
        ui.painter().rect_filled(rect, 0.0, contrast);
        if self.channel != index {
            // Another channel's rack: a key-learn ends on the channel it was on.
            if let Some((id, _)) = self.learning {
                actions.setting_changes.push(learn(self.channel, id, false));
            }
            // What the host set for this frame stays, and so does a pending note-effect rebuild: a keyboard switch of
            // the current channel mid-drag still builds the edit in once the pointer is up.
            let panels = std::mem::take(&mut self.panels);
            *self = Self {
                channel: index,
                panels,
                note_edit: self.note_edit,
                meters: std::mem::take(&mut self.meters),
                soundfonts: std::mem::take(&mut self.soundfonts),
                targets: std::mem::take(&mut self.targets),
                covered: self.covered,
                multitrack: self.multitrack,
                input_peaks: self.input_peaks,
                middle_c_is_c4: self.middle_c_is_c4,
                ..Self::default()
            };
        }
        self.panels.retain(|&(ch, id, ty), _| {
            ch != index || channel.rack.get(id).is_some_and(|m| m.type_id == ty)
        });

        // Scrolling first: the content is placed from the new offset, by the editors' law: the controls get the touch
        // first, as they are on top.
        // While a card is held, every card shows minimized.
        let held = self.drag.map(|d| d.id);
        let heights: Vec<f32> = (channel.rack.modules.iter())
            .map(|m| held.map_or(card_height(m), |_| CARD_HEADER))
            .collect();
        let sends = (channel.kind != ChannelKind::Master).then(|| SendRows {
            rows: channel.file.sends.len(),
            disconnected: disconnected(channel, &self.targets),
        });
        let input = channel.kind == ChannelKind::AudioRecord;
        let end = flow_sends(rack, 0.0, u, input, &heights, sends).bottom;
        // One axis, y in points over the whole rack. Not while a card is held: neither its finger nor the wheel scrolls
        // (the card steps instead), and the offset is not clamped, keeping the card in place as the cards shrink and
        // grow, until the scroller lets go of the touch with the hold and springs back from there.
        let axes = |scroll: f32| {
            let y = Axis {
                pos: f64::from(scroll),
                zoom: 1.0,
                range: [0.0, f64::from((end - rack.top()) / ppp)],
                zoom_range: [1.0; 2],
                view: rect.y_range(),
            };
            [Axis::OFF, y]
        };
        let scroll_id = ui.id().with("rack");
        if held.is_none() {
            let zones = Zones {
                touch: [Rect::NOTHING, rect],
                wheel_scroll: rect,
                ..Zones::NONE
            };
            // No new touch or wheel under an overlay (an open menu or dialog's backdrop is one), nor in a rack sliding
            // out, which is drawn disabled.
            let opts = scroller::Options {
                covered: self.covered || !ui.is_enabled(),
                follow: false,
            };
            let mut axes = axes(self.scroll);
            (self.scroller).update(ui, scroll_id, m.u(1.0), &zones, &mut axes, opts);
            self.scroll = axes[Y].pos as f32;
        }
        // Clicks that nothing on the rack takes are the rack's (not a control a few points off, such as the notch).
        ui.interact(rect, ui.id().with("rack clicks"), Sense::click());
        let at = (self.scroll * ppp).round();
        let f = flow_sends(rack, at, u, input, &heights, sends);

        // The scrolling content slides under the header bar.
        let outer = ui.clip_rect();
        ui.set_clip_rect(outer.intersect(rect.with_min_y(rect.top() + m.u(HEADER_BAR))));
        let (mut confirmed, mut held_rect) = (None, None);
        // The held card draws last, over a neighbour sliding past it.
        let mut cards: Vec<_> = channel.rack.modules.iter_mut().zip(&f.cards).collect();
        cards.sort_by_key(|(m, _)| held == Some(m.instance_id));
        for (module, card) in cards {
            let id = module.instance_id;
            let slide = self.slides.get(&id).map_or(0.0, |o| o * u);
            let card = card.translate(vec2(0.0, slide)) / ppp;
            // The held card goes on taking its press wherever it is.
            if held != Some(id) && !ui.clip_rect().intersects(card) {
                continue;
            }
            let CardEdits {
                changed,
                stored,
                presets,
                press,
                sample,
                auto,
                touch,
                sent,
                header,
                double_tap,
            } = self.card(
                ui,
                cx,
                card,
                module,
                held.is_some(),
                ink,
                if input { 1 } else { 2 },
            );
            // The name label's gestures: a double tap is the menu's `Minimize` / `Maximize`, a long press holds the
            // card, and its release lets go.
            if double_tap {
                module.minimized ^= true;
                actions.minimized = true;
            }
            if header.long && header.held {
                let origin = ui.input(|i| i.pointer.press_origin());
                self.drag = Some(Drag {
                    id,
                    origin: origin.unwrap_or(card.center()),
                    started: false,
                    stepped: None,
                });
                confirmed = Some(id);
            } else if !header.held && self.drag.is_some_and(|d| d.id == id) {
                self.drag = None;
            }
            if held == Some(id) {
                held_rect = Some(card);
            }
            actions.settings |= stored;
            actions.touch = touch.or(actions.touch);
            // A setting the engine derives app-thread data from at build rebuilds it once the pointer is up, as a
            // note-effect edit does; the others go out live.
            let (id, type_id) = (module.instance_id, module.type_id);
            let (rebuild, live): (Vec<_>, Vec<_>) =
                (sent.into_iter()).partition(|&(key, _)| settings::rebuilds(type_id, key));
            self.note_edit |= !rebuild.is_empty();
            if type_id == slicer::TYPE_ID && !rebuild.is_empty() && !self.redetect.contains(&id) {
                self.redetect.push(id);
            }
            if auto {
                actions.auto_clip = Some(id);
            }
            actions
                .setting_changes
                .extend(live.into_iter().map(|(key, value)| SettingChange {
                    channel: index,
                    instance_id: id,
                    key,
                    value,
                }));
            if presets {
                actions.picker = Some(PresetTarget::Module(id));
            }
            match press {
                Some(Press::Picker) => actions.soundfont = Some(module.instance_id),
                // Opened in `card`.
                Some(Press::Menu) => {}
                // Another patch: a final edit, as a picked one is.
                Some(Press::Stepped) => actions.replace_song = true,
                None => {}
            }
            if sample || three_osc::custom_picked(module, &changed) {
                actions.module_sample = Some(module.instance_id);
            }
            self.report(&mut actions, index, module, changed);
        }
        if let Some(row) = f.input {
            let row = InputRow {
                row: row / ppp,
                rack_width: rect.width(),
                ink,
                contrast,
                monitoring: channel.file.input_monitoring,
                armed: channel.file.record_armed,
                multitrack: self.multitrack,
                peaks: self.input_peaks,
            };
            self.input(ui, cx, &row, channel, &mut actions);
        }
        let add = f.add_module / ppp;
        add_row(ui, cx, add, "Add Module", ink, Some(contrast));
        if ui
            .interact(add, ui.id().with("add module"), Sense::click())
            .clicked()
        {
            actions.open_browser = Some(InsertAt::End);
        }
        if let Some(sends) = &f.sends {
            self.sends(ui, cx, sends, index, channel, ink, &mut actions);
        }
        // The held card's 5 px outline in the channel colour, outside its rect, over its neighbours.
        if let (Some(r), Some(_)) = (held_rect, self.drag) {
            let stroke = Stroke::new(CARD_GAP_PX / ppp, ink);
            ui.painter()
                .rect_stroke(r, 0.0, stroke, StrokeKind::Outside);
        }
        ui.set_clip_rect(outer);
        // The swap slides decay once a drawn frame.
        let frame = ui.ctx().cumulative_frame_nr();
        if self.slid != frame {
            self.slid = frame;
            self.slides.retain(|_, o| {
                *o *= 0.7;
                (*o * u).abs() >= 1.0
            });
        }
        let view = rack.with_min_y(rack.top() + HEADER_BAR * u);
        self.step(ui, cx, &f, view, held_rect, &mut channel.rack, &mut actions);
        // A hold confirmed: every card shows minimized from the next frame on, and the offset moves by the held card's
        // top shift so that it stays put. Let go: they show as they are again, and the offset follows the held card if
        // its new top leaves [p, p + 0.9·H].
        let modules = &channel.rack.modules;
        let slot = |id| modules.iter().position(|m| m.instance_id == id);
        if let Some(i) = confirmed.and_then(slot) {
            let minimized = vec![CARD_HEADER; modules.len()];
            let top = flow_sends(rack, at, u, input, &minimized, sends).cards[i].top();
            self.scroll += (top - f.cards[i].top()) / ppp;
        }
        if let (Some(id), None) = (held, self.drag) {
            if let Some(i) = slot(id) {
                let shown: Vec<f32> = modules.iter().map(card_height).collect();
                let top = flow_sends(rack, at, u, input, &shown, sends).cards[i].top();
                let y = top - view.top();
                if y < at || y > at + 0.9 * view.height() {
                    self.scroll += (top - f.cards[i].top()) / ppp;
                }
            }
            // Out of the held layout's range springs back (the full layout's range holds it: the spring may find
            // nothing to do).
            self.scroller.let_go(&axes(self.scroll));
        }

        actions.close = self.chrome(ui, cx, rack, &f, &channel.name, ink, contrast);
        self.popups(ui, cx, index, channel, &mut actions);
        // A control that took the drag (an Equalizer handle claims it), or a popup open over the rack (a long press's
        // dialog with the finger still down), has the touch: the scroller yields it, as to a held track header, and no
        // scroll, tap or fling comes of it. (A held card's touch the scroller does not see: it lets go of it with the
        // hold.)
        let taken = ui.ctx().dragged_id().is_some_and(|id| id != scroll_id);
        let popup = self.card_menu.is_some()
            || self.rack_menu.is_some()
            || self.dialog.is_some()
            || self.send_menu.is_some()
            || self.patch_menu.is_some()
            || self.input_menu.is_some();
        if taken || popup {
            self.scroller.yield_touch();
        }
        self.learn(index, channel, &mut actions);
        if self.note_edit && !ui.input(|i| i.pointer.any_down()) {
            actions.replace_song = true;
            actions.redetect = std::mem::take(&mut self.redetect);
            self.note_edit = false;
        }
        if !self.slides.is_empty() {
            ui.ctx().request_repaint();
        }
        actions
    }

    /// The held card's step rule. Nothing runs until the finger first moves more than 0.5 U vertically from where it
    /// went down; then the rule runs at once and on each move event after, never on a timer, and a run within d of the
    /// last step is dropped. A run reads the card's drawn rect (`held`, points: its layout plus its slide); a step is
    /// the card menu's `Move up` / `Move down`, which slides, and with y′ the card's new top, the offset p moves by
    /// y′ − y when y′ is out of [p, p + 0.9·H] (y′ measured down the viewport `view`, H its height, in px). A release,
    /// a focus loss or a second finger lets go.
    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        ui: &Ui,
        cx: &UiCtx,
        f: &Flow,
        view: Rect,
        held: Option<Rect>,
        rack: &mut Rack,
        actions: &mut RackActions,
    ) {
        let Some(mut drag) = self.drag else {
            return;
        };
        let (down, cancel, finger, moved, now) = ui.input(|i| {
            let moved = (i.events.iter()).any(|e| matches!(e, Event::PointerMoved(_)));
            let cancel = focus_lost(i) || i.multi_touch().is_some();
            let p = &i.pointer;
            (p.primary_down(), cancel, p.latest_pos(), moved, i.time)
        });
        if !down || cancel {
            self.drag = None;
            return;
        }
        // The frame that confirms the hold has no held rect yet.
        let (Some(card), Some(finger)) = (held, finger) else {
            return;
        };
        let (m, ppp) = (cx.m, cx.m.ppp());
        if !drag.started {
            if gesture::vertical(finger - drag.origin) <= m.u(gesture::CALM_U) {
                return;
            }
            drag.started = true;
            self.drag = Some(drag);
        } else if !moved {
            return;
        }
        if drag.stepped.is_some_and(|(t, d)| now - t < d) {
            return;
        }
        let gap = CARD_GAP_PX / ppp;
        let Some(action) = drag_step(card.top(), finger.y, m.u(1.0), gap) else {
            return;
        };
        if self.act(rack, drag.id, action) == Outcome::ChainChanged {
            actions.replace_song = true;
        }
        // Held, every card is 1.5 U: the new layout is this pass's, slot for slot.
        let Some(i) = rack.index_of(drag.id) else {
            return;
        };
        let top = f.cards[i].top();
        let (p, y) = ((self.scroll * ppp).round(), top - view.top());
        if y < p || y > p + 0.9 * view.height() {
            self.scroll += top / ppp - card.top();
        }
        drag.stepped = Some((now, step_interval(top / ppp, finger.y, m.u(1.0))));
        self.drag = Some(drag);
        ui.ctx().request_repaint();
    }

    /// [`apply`], and the swap slide of a `Move up` / `Move down` that swapped, the menu's and a held card's: the moved
    /// card starts offset toward its old place by its neighbour's shown height (U), the neighbour by the moved card's
    /// the other way.
    fn act(&mut self, rack: &mut Rack, id: i32, action: CardAction) -> Outcome {
        let from = rack.index_of(id);
        let outcome = apply(rack, id, action);
        let swap = matches!(action, CardAction::MoveUp | CardAction::MoveDown);
        if let (true, Outcome::ChainChanged, Some(from), Some(to)) =
            (swap, outcome, from, rack.index_of(id))
        {
            let shown = |m: &ModuleInstance| match self.drag {
                Some(_) => CARD_HEADER,
                None => card_height(m),
            };
            let (moved, other) = (&rack.modules[to], &rack.modules[from]);
            let toward = if to < from { 1.0 } else { -1.0 };
            let offsets = [
                (id, toward * shown(other)),
                (other.instance_id, -toward * shown(moved)),
            ];
            self.slides.extend(offsets);
        }
        outcome
    }

    /// The keys played since the last pass, for the running key-learn ([`Self::key_played`]). A learn whose module is
    /// gone just ends: its engine is rebuilt without it.
    fn learn(&mut self, index: usize, channel: &mut Channel, actions: &mut RackActions) {
        for key in std::mem::take(&mut self.played) {
            let Some((id, low)) = self.learning else {
                break;
            };
            let Some(module) = channel.rack.get_mut(id) else {
                self.learning = None;
                break;
            };
            let file = &mut module.file;
            let key = key as i8;
            match low {
                _ if instrument(module.type_id) == Some(SynthKind::Slicer) => file.key_low = key,
                Some(low) => {
                    let low = low as i8;
                    (file.key_low, file.key_high) = (low.min(key), low.max(key));
                }
                None => {
                    self.learning = Some((id, Some(key as u8)));
                    continue;
                }
            }
            self.learning = None;
            actions.setting_changes.push(learn(index, id, false));
            actions.replace_song = true;
        }
    }

    /// The SENDS section in `f`: the header, one row per `SEND` of `channel` (the song's channel `index`) and
    /// `Add Send`, then `Disconnected from MASTER` when it applies. A row's toggle and slider are its Enable Send and
    /// Send Level (channel params 0x1000 / 0x2000 + the target's id), live param edits; its swap icon retargets it and
    /// its X removes it, final edits (the routing changes). A removed send's automation lanes stay, inert until a send
    /// to that channel is back.
    #[allow(clippy::too_many_arguments)]
    fn sends(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        f: &SendsFlow,
        index: usize,
        channel: &mut Channel,
        ink: Color32,
        actions: &mut RackActions,
    ) {
        let m = cx.m;
        let (ppp, text) = (m.ppp(), cx.theme.colors.text.to_color32());
        let header = f.header / ppp;
        let (icon, label) = header.split_left_right_at_x(header.left() + header.width() / 10.0);
        draw_icon(ui.painter(), icon, IconId::Routing, text, 1.0, m);
        widgets::text_label(
            ui,
            cx,
            label,
            "SENDS",
            FontSize::Pt10,
            Align2::LEFT_CENTER,
            text,
        );
        let base = ui.id().with("sends");
        let mut remove = None;
        // The rows' params are channel params: their `Unlink from MIDI CC` follows the channel's `CCLK`.
        let linked = links::links(&channel.file.cc_links);
        let menu = |param| ParamMenu::bound(links::has(&linked, 0, param));
        for (i, (send, &row)) in channel.file.sends.iter_mut().zip(&f.rows).enumerate() {
            let [toggle, swap, name, level, x] = send_row(ui, cx, row / ppp);
            let target = send.target_channel_id;
            let id = base.with(i);
            let mut control = |param: i32, value: &mut f32, changed: bool, signal| {
                if changed {
                    actions.params.push(ParamChange {
                        channel: index,
                        instance_id: 0,
                        index: param,
                        value: *value,
                    });
                }
                let target = AutomationTarget {
                    instance_id: 0,
                    param,
                };
                actions.touch = Touch::new(target, signal).or(actions.touch);
            };
            let c = Control {
                menu: menu(0x1000 + target),
                ..Control::new(id.with("enable"), toggle, "", ink, 1.0)
            };
            let changed =
                widgets::toggle(ui, cx, &c, &mut send.enabled, |_| String::new()).changed();
            control(
                0x1000 + target,
                &mut send.enabled,
                changed,
                widgets::signal(ui, c.id),
            );
            let c = Control {
                menu: menu(0x2000 + target),
                fine_tune: crate::ui::fine_tune::channel_param(0x2000 + target),
                ..Control::new(id.with("level"), level, "", ink, SEND_UNITY)
            };
            let changed =
                widgets::h_slider(ui, cx, &c, &mut send.level, |_| String::new()).changed();
            control(
                0x2000 + target,
                &mut send.level,
                changed,
                widgets::signal(ui, c.id),
            );
            if icon_button(ui, cx, id.with("swap"), swap, IconId::Swap, ink) {
                self.send_menu = Some((Some(i), swap.center()));
            }
            let named = self.targets.iter().find(|t| t.id == target);
            // Without the host's names: the master's, else the name a new send channel gets.
            let fallback = || match target {
                0 => "MASTER".to_owned(),
                id => format!("Effect {id}"),
            };
            let name_text = named.map_or_else(fallback, |t| t.name.clone());
            widgets::text_label(
                ui,
                cx,
                name,
                &name_text,
                FontSize::Pt10,
                Align2::CENTER_CENTER,
                ink,
            );
            if icon_button(ui, cx, id.with("remove"), x, IconId::Close, ink) {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            channel.file.sends.remove(i);
            self.settle_output(channel);
            actions.replace_song = true;
        }
        // The four trailing spaces shift the group left.
        let add = f.add / ppp;
        add_row(ui, cx, add, "Add Send    ", ink, None);
        if ui.interact(add, base.with("add"), Sense::click()).clicked() {
            self.send_menu = Some((None, add.center()));
        }
        if let Some(row) = f.disconnected {
            let label = "Disconnected from MASTER";
            widgets::text_label(
                ui,
                cx,
                row / ppp,
                label,
                FontSize::Pt10,
                Align2::CENTER_CENTER,
                text,
            );
        }
    }

    /// `channel`'s output bus as its sends now say (`RHED`[12]), by the targets' buses.
    fn settle_output(&self, channel: &mut Channel) {
        let buses: Vec<(i32, i32)> = (self.targets.iter())
            .filter(|t| t.bus >= 1)
            .map(|t| (t.id, t.bus))
            .collect();
        channel.output_bus = output_bus(&channel.file.sends, &buses);
    }

    /// The open send-target menu, one column: the offered targets the channel does not send to yet (for a swap, its own
    /// target shown current), then `Create FX Channel`. A pick adds a send at the defaults, or retargets the swapped
    /// one keeping its Enable and Level; a final edit.
    fn send_menu(&mut self, ui: &Ui, cx: &UiCtx, channel: &mut Channel, actions: &mut RackActions) {
        let Some((swap, at)) = self.send_menu else {
            return;
        };
        let sends = &channel.file.sends;
        let own = swap.and_then(|i| sends.get(i)).map(|s| s.target_channel_id);
        let free = |t: &&SendTarget| {
            t.offered && (Some(t.id) == own || !sends.iter().any(|s| s.target_channel_id == t.id))
        };
        let mut items: Vec<_> = (self.targets.iter().filter(free).enumerate())
            .map(|(row, t)| PopupItem {
                current: Some(t.id) == own,
                ..PopupItem::new(SendPick::Target(t.id), 0, row, t.name.clone())
            })
            .collect();
        items.push(PopupItem::new(
            SendPick::Create,
            0,
            items.len(),
            "Create FX Channel",
        ));
        let ctx = ui.ctx();
        let r = popup::show(
            ctx,
            ui.id().with("send menu"),
            at,
            ctx.content_rect(),
            &items,
            cx,
        );
        if r.close {
            self.send_menu = None;
            ctx.request_repaint();
        }
        let at = swap.unwrap_or(channel.file.sends.len());
        match r.selected {
            Some(SendPick::Create) => actions.create_send = Some(at),
            Some(SendPick::Target(id)) if Some(id) != own => {
                match channel.file.sends.get_mut(at) {
                    Some(send) => send.target_channel_id = id,
                    None => channel.file.sends.push(SendState {
                        target_channel_id: id,
                        ..SendState::default()
                    }),
                }
                self.settle_output(channel);
                actions.replace_song = true;
            }
            _ => {}
        }
    }

    /// Adds the params `changed` on `module` to `actions`; a note effect's also call for a new song.
    fn report(
        &mut self,
        actions: &mut RackActions,
        channel: usize,
        module: &ModuleInstance,
        changed: Vec<i32>,
    ) {
        let note = module_spec(module.type_id).is_some_and(|s| s.role == Role::NoteEffect);
        for index in changed {
            self.note_edit |= note;
            actions.params.push(ParamChange {
                channel,
                instance_id: module.instance_id,
                index,
                value: module.param(index).unwrap_or_default(),
            });
        }
    }

    /// One card in `card` (points): background, header row and, unless minimized (or `minimized`, every card while one
    /// is held), the module panel. `channel_kind` is the panel's flag of that name: 1 on an audio-input channel, 2 on
    /// any other.
    #[allow(clippy::too_many_arguments)]
    fn card(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        card: Rect,
        module: &mut ModuleInstance,
        minimized: bool,
        ink: Color32,
        channel_kind: usize,
    ) -> CardEdits {
        let (m, colors) = (cx.m, &cx.theme.colors);
        let (ppp, u) = (m.ppp(), m.u_px());
        let (text, fill) = (colors.text.to_color32(), colors.module_fill.to_color32());
        let id = module.instance_id;
        let base = ui.id().with(("rack card", id));
        let painter = ui.painter();
        painter.rect_filled(card, 0.0, fill);
        // Both 1 px rules inside the card.
        let rule = 1.0 / ppp;
        painter.rect_filled(card.with_max_y(card.top() + rule), 0.0, ink);
        painter.rect_filled(card.with_min_y(card.bottom() - rule), 0.0, ink);

        let preset = module.file.preset_name.clone();
        let preset_w = preset_width(text_width(ui, &preset, m.font_id(FontSize::Pt10)), m.u(1.0));
        let [toggle, icon, name, preset_rect, menu] =
            header_row(card * ppp, u, preset_w).map(|r| r / ppp);
        // The module-name label (its cell in the header row) takes the card's own gestures, not the header's buttons.
        let header_id = base.with("header");
        let mut held = ui.interact(name, header_id, Sense::click());
        let header = gesture::press(ui, &mut held, m, gesture::vertical);
        let double_tap = widgets::double_tap(ui, header_id, &held);
        let (mut changed, mut stored, mut press) = (Vec::new(), false, None);
        let (mut sent, mut sample, mut auto) = (Vec::new(), false, false);

        // Enable, p0. A module without params (an unknown type) shows it on and inert.
        let has_enable = !module.params.is_empty();
        let mut on = 1.0;
        let enable = module.params.first_mut().unwrap_or(&mut on);
        let linked = links::has(&panels::cc_links(ui), id, 0);
        let c = Control {
            disabled: !has_enable,
            menu: ParamMenu::bound(linked),
            ..Control::new(base.with("enable"), toggle, "", ink, 1.0)
        };
        if widgets::toggle(ui, cx, &c, enable, |_| String::new()).changed() {
            changed.push(0);
        }
        let target = |param| AutomationTarget {
            instance_id: id,
            param,
        };
        let mut touch = Touch::new(target(0), widgets::signal(ui, c.id));
        // The header's icon buttons: a `text` icon on a `moduleFill` background, which is invisible on the card. A disc
        // would cut the icon down to fit it (`Button`'s rule), so the background is the rect and the icon a fixed size.
        let icon_button = |id: Id, rect: Rect, icon: IconId| Button {
            icon: Some(icon),
            icon_scale: HEADER_ICON,
            fill_on: fill,
            fill_off: fill,
            text_on: text,
            text_off: Some(text),
            ..Button::new(id, rect, "", false, colors)
        };
        let b = icon_button(base.with("icon"), icon, module_icon(module.type_id));
        let mut r = widgets::button(ui, cx, &b, true);
        // An instrument's level popup opens on a long press (mouse or touch) or a secondary click only; an
        // effect's dialog on a plain click too.
        let effect = module_spec(module.type_id).is_some_and(|s| s.role != Role::Instrument);
        let long = gesture::long_press(ui, &mut r, m);
        if r.clicked_by(eframe::egui::PointerButton::Secondary) || long || (effect && r.clicked()) {
            self.dialog = Some((id, icon.center()));
        }
        // Truncated, keeping 5 tail characters.
        let (label, font) = (module_name(module.type_id), m.font_id(FontSize::Pt12));
        widgets::truncated(
            ui.painter(),
            name,
            Align2::LEFT_CENTER,
            label,
            font,
            text,
            5,
        );
        preset_button(ui, cx, preset_rect, &preset, text);
        let presets = (ui.interact(preset_rect, base.with("preset"), Sense::click())).clicked();
        let b = icon_button(base.with("menu"), menu, IconId::Menu);
        if widgets::button(ui, cx, &b, true).clicked() {
            self.card_menu = Some((id, menu.center()));
        }

        if let (false, Some(h)) = (module.minimized || minimized, ui_height(module.type_id)) {
            let content = content_rect(card * ppp, h, u) / ppp;
            match panels::panel_for(module.type_id) {
                Some(panel) => {
                    let key = (self.channel, id, module.type_id);
                    let fresh = !self.panels.contains_key(&key);
                    let card = self.panels.entry(key).or_insert_with(|| Card {
                        state: PanelState::new(panel),
                        cstm: None,
                    });
                    if fresh || card.cstm != module.file.custom_data {
                        cstm::seed(module, &mut card.state);
                        card.cstm.clone_from(&module.file.custom_data);
                    }
                    let state = &mut card.state;
                    // Left absent (0): `constructing` (SuperSaw), which an updated card never sets. The SoundFont
                    // Player's wiring sets `loading`, the Slicer's `sample_loaded`.
                    state.flags.insert(panels::flag::CHANNEL_KIND, channel_kind);
                    let offset = usize::from(self.middle_c_is_c4);
                    state.flags.insert(panels::flag::OCTAVE_OFFSET, offset);
                    let soundfont = module.type_id == soundfont_player::TYPE_ID;
                    if soundfont {
                        soundfont_player::wire(module, state, &self.soundfonts);
                    }
                    if module.type_id == three_osc::TYPE_ID {
                        three_osc::wire(module, state);
                    }
                    if module.type_id == slicer::TYPE_ID {
                        slicer::wire(module, state);
                    }
                    let before = (state.settings.clone(), state.tab);
                    // The module's meter for the panel's draws, silence when the audio has none for it; none without
                    // a running stream, so the draws rest and stop polling.
                    let key = ModuleKey {
                        channel: self.channel,
                        instance_id: id,
                    };
                    let feed = (self.meters.as_ref()).map(|meters| {
                        let meter = meters.iter().find(|m| m.key == key);
                        meter.map_or((MeterSnapshot::default(), &[][..]), |m| {
                            (m.value, &m.analysis[..])
                        })
                    });
                    panels::post_feed(ui, id, feed);
                    // A Note Echo's `Stop` is enabled while echoes are pending.
                    if module.type_id == NOTE_ECHO {
                        let slab = feed.map(|(_, slab)| slab);
                        let pending =
                            slab.and_then(honeydew_audio::analysis::note_echo) == Some(true);
                        state
                            .flags
                            .insert(panels::flag::PENDING, usize::from(pending));
                    }
                    let edits = panels::show(ui, cx, panel, module, state, content, ink);
                    let pressed = edits.pressed.map(|p| (p, None));
                    let picked = edits.pick.map(|(p, pick)| (p, Some(pick)));
                    if let Some((p, pick)) = picked.or(pressed) {
                        touch = Some(Touch {
                            target: target(p as i32),
                            pick,
                        });
                    }
                    // A user edit stores the settings and page, with what the panel's hook set (Pitcher 2's Scale
                    // after a note edit). A hook's own correction (the Tuner's Input off) stays in the card until then.
                    let edited = !edits.settings.is_empty() || !edits.params.is_empty();
                    let moved = state.settings != before.0 || state.tab != before.1;
                    changed.extend(edits.params.into_iter().map(|p| p as i32));
                    if edited && moved && cstm::store(module, state) {
                        card.cstm.clone_from(&module.file.custom_data);
                        stored = true;
                    }
                    // The engine gets what a build would read: the settings a user edit just stored in the blob (the
                    // hook's among them: the Pitcher's notes after a Scale change), and a Multiband's soloed band, UI
                    // state. Not a card's seeding (the new engine reads that blob), a hook's correction nothing
                    // stored (the Tuner's Input off), nor a setting a hook adds on the card's first pass, except a
                    // screen-only one: no build reads it, so a card that starts without it (a clone's,
                    // [`Self::copy_panels`]) sends it as its hook sets it.
                    let screen_only = |k: u32| settings::screen_only(module.type_id, k);
                    let settings = state.settings.iter().map(|(&k, &v)| (k, v));
                    sent.extend(settings.filter(|&(k, v)| {
                        let changed = before.0.get(&k).map_or(screen_only(k), |&b| b != v);
                        (stored || screen_only(k)) && changed
                    }));
                    sent.sort_by_key(|&(key, _)| key);
                    if module.type_id == slicer::TYPE_ID {
                        let (id, u) = (base.with("loop"), cx.m.u(1.0));
                        sample = slicer::asks_for_loop(
                            ui,
                            id,
                            module,
                            state,
                            content,
                            u,
                            &edits.settings,
                        );
                        auto = slicer::auto_pressed(state, &edits.settings);
                    }
                    if soundfont {
                        let fonts = &self.soundfonts;
                        press = soundfont_player::press(module, state, &edits.settings, fonts)
                            .or_else(|| {
                                let id = base.with("soundfont error");
                                soundfont_player::error_tap(ui, id, content, state)
                            });
                        // The patch menu is the rack's own popup, over the spinner.
                        if press == Some(Press::Menu) {
                            let u = cx.m.u(1.0);
                            let at =
                                soundfont_player::menu_anchor(panel, module, state, content, u);
                            self.patch_menu = Some((id, at));
                        }
                    }
                }
                None => widgets::display_area(ui, cx, content),
            }
        }
        CardEdits {
            changed,
            stored,
            presets,
            press,
            sample,
            auto,
            touch,
            sent,
            header,
            double_tap,
        }
    }

    /// The fixed chrome over the content: header bar with the channel name, the rack ≡, the divider and the
    /// notch. `rack` is in device pixels. Returns whether the notch was clicked.
    #[allow(clippy::too_many_arguments)]
    fn chrome(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        rack: Rect,
        f: &Flow,
        name: &str,
        ink: Color32,
        contrast: Color32,
    ) -> bool {
        let m = cx.m;
        let (ppp, u) = (m.ppp(), m.u_px());
        let text = cx.theme.colors.text.to_color32();
        let painter = ui.painter();
        let bar_bottom = (rack.top() + HEADER_BAR * u - 2.0).floor();
        let bar = Rect::from_x_y_ranges(f.add_module.x_range(), rack.top()..=bar_bottom);
        painter.rect_filled(bar / ppp, 0.0, ink);
        let line = Rect::from_x_y_ranges(f.add_module.x_range(), bar_bottom..=bar_bottom + 2.0);
        painter.rect_filled(line / ppp, 0.0, contrast);
        widgets::text_label(
            ui,
            cx,
            bar / ppp,
            name,
            FontSize::Pt10,
            Align2::CENTER_CENTER,
            contrast,
        );

        ui.painter().rect_filled(divider(rack) / ppp, 0.0, ink);
        let notch = notch(rack, u);
        ui.painter().add(Shape::convex_polygon(
            notch.map(|p| p / ppp).to_vec(),
            ink,
            Stroke::NONE,
        ));

        let at = (rack.min / ppp) + Vec2::splat(m.u(GUTTER / 2.0));
        let id = ui.id().with("rack menu button");
        if outline_circle_button(ui, id, at, m.u(RACK_MENU_D), text, m).clicked() {
            self.rack_menu = Some(at);
        }
        draw_icon(
            ui.painter(),
            Rect::from_center_size(at, Vec2::ZERO),
            IconId::Menu,
            text,
            RACK_MENU_ICON,
            m,
        );
        // The hit area is the left gutter's width (it holds no cards) × 1 U.
        let notch_rect = Rect::from_min_max(
            pos2(rack.left(), notch[0].y),
            pos2(rack.left() + GUTTER * u, notch[2].y),
        );
        ui.interact(notch_rect / ppp, ui.id().with("notch"), Sense::click())
            .clicked()
    }

    /// The open card menu, rack menu or dialog, and what picking from them does.
    fn popups(
        &mut self,
        ui: &Ui,
        cx: &UiCtx,
        index: usize,
        channel: &mut Channel,
        actions: &mut RackActions,
    ) {
        self.send_menu(ui, cx, channel, actions);
        let ctx = ui.ctx();
        let screen = ctx.content_rect();
        if let Some((id, at)) = self.card_menu {
            let Some(module) = channel.rack.get(id) else {
                self.card_menu = None;
                return;
            };
            let items = card_menu(module.minimized, module.type_id, full_range(module));
            let menu_id = ui.id().with(("rack card menu", id));
            let r = popup::show(ctx, menu_id, at, screen, &items, cx);
            if r.close {
                self.card_menu = None;
                ctx.request_repaint();
            }
            let automated =
                (channel.lanes.iter()).any(|l| l.target.is_some_and(|t| t.instance_id == id));
            let selected = match r.selected {
                Some(CardAction::Delete) if automated => {
                    actions.ask_delete = Some(id);
                    None
                }
                selected => selected,
            };
            if selected == Some(CardAction::SetRange) {
                // One key-learn at a time: another card's ends.
                if let Some((other, _)) = self.learning.replace((id, None)) {
                    actions.setting_changes.push(learn(index, other, false));
                }
                self.played.clear();
                actions.setting_changes.push(learn(index, id, true));
            }
            match selected.map(|action| self.act(&mut channel.rack, id, action)) {
                Some(Outcome::ChainChanged) => actions.replace_song = true,
                Some(Outcome::Browse(at)) => actions.open_browser = Some(at),
                Some(Outcome::Minimized) => actions.minimized = true,
                Some(Outcome::Copied) => {
                    actions.copy = channel.rack.get(id).cloned().map(|m| vec![m])
                }
                Some(Outcome::SavePreset) => {
                    actions.preset = channel.rack.get(id).cloned().map(PresetSource::Module)
                }
                _ => {}
            }
        }
        if let Some((id, at)) = self.patch_menu {
            let fonts = &self.soundfonts;
            let items = channel
                .rack
                .get(id)
                .and_then(|m| soundfont_player::menu_items(m, fonts));
            match items {
                Some(items) => {
                    let menu_id = ui.id().with(("rack patch menu", id));
                    let r = popup::show(ctx, menu_id, at, screen, &items, cx);
                    if r.close {
                        self.patch_menu = None;
                        ctx.request_repaint();
                    }
                    let module = channel.rack.get_mut(id);
                    let pick = r.selected.zip(module);
                    let pick = pick.and_then(|(i, m)| soundfont_player::menu_pick(m, fonts, i));
                    actions.replace_song |= pick == Some(Press::Stepped);
                }
                // Gone with its card, its SoundFont or its patches (a delete, a pick): closed.
                None => self.patch_menu = None,
            }
        }
        if let Some(at) = self.input_menu {
            // Shown once the host has listed the devices, after the frame that opened it.
            let close = match &self.inputs {
                None => {
                    ctx.request_repaint();
                    false
                }
                Some((devices, current)) => {
                    let items = input_items(devices, *current);
                    let r = popup::show(ctx, ui.id().with("input menu"), at, screen, &items, cx);
                    // `<no input>` and the device rows pick nothing.
                    if let Some(Some(choice)) = r.selected {
                        actions.input = Route::picked(choice, devices);
                    }
                    r.close
                }
            };
            if close {
                (self.input_menu, self.inputs) = (None, None);
                ctx.request_repaint();
            }
        }
        if let Some(at) = self.rack_menu {
            let drums = channel.kind == ChannelKind::Drums;
            let r = popup::show(
                ctx,
                ui.id().with("rack menu"),
                at,
                screen,
                &rack_menu(drums, channel.rack.modules.is_empty()),
                cx,
            );
            if r.close {
                self.rack_menu = None;
                ctx.request_repaint();
            }
            match r.selected {
                Some(item @ (RackItem::MinimizeAll | RackItem::MaximizeAll)) => {
                    for module in &mut channel.rack.modules {
                        module.minimized = item == RackItem::MinimizeAll;
                    }
                    actions.minimized = true;
                }
                Some(RackItem::ClearRack) => {
                    let before = channel.clone();
                    if clear(&mut channel.rack) == Outcome::ChainChanged {
                        actions.ask_remove = hold_removal(channel, before);
                        actions.replace_song |= actions.ask_remove.is_none();
                    }
                }
                Some(RackItem::LoadPreset) => actions.picker = Some(PresetTarget::RackReplace),
                Some(RackItem::LoadDrumset) => actions.picker = Some(PresetTarget::Drumset),
                Some(RackItem::CopyRack) => actions.copy = Some(channel.rack.modules.clone()),
                Some(RackItem::SavePreset) => {
                    actions.preset = Some(PresetSource::Rack {
                        name: channel.name.clone(),
                        modules: channel.rack.modules.clone(),
                    });
                }
                _ => {}
            }
        }
        if let Some((id, at)) = self.dialog {
            let Some(module) = channel.rack.get_mut(id) else {
                self.dialog = None;
                return;
            };
            let (changed, close, touch) =
                common_dialog(ui, cx, ui.id().with(("rack dialog", id)), at, module);
            actions.touch = touch.or(actions.touch);
            self.report(actions, index, module, changed);
            // A slider's `Fine tune control` hands over to the fine tune dialog, which opens over the screen: this one
            // closes, or it would stay live above that dialog's dim.
            let fine_tune = touch.is_some_and(|t| t.pick == Some(CtrlAction::FineTune));
            if close || fine_tune {
                self.dialog = None;
                ctx.request_repaint();
            }
        }
    }
}

/// The header row's items in card `card` (device pixels, as `u`): enable toggle, icon, name, preset button and ≡.
/// `preset_w` is the preset button's width in U.
fn header_row(card: Rect, u: f32, preset_w: f32) -> [Rect; 5] {
    let (px8, px4) = (8.0 / u, 4.0 / u);
    let fixed = |id, w| Node::leaf(id).w(Size::U(w)).margins(px8, 0.0, px8, 0.0);
    let row = Node::hstack([
        fixed(0, 1.5),
        fixed(1, 1.5),
        Node::leaf(2).margins(px4, 0.0, px4, 0.25),
        fixed(3, preset_w),
        fixed(4, 1.5),
    ]);
    let rects = layout::layout(&row, card.with_max_y(card.top() + CARD_HEADER * u), u);
    std::array::from_fn(|i| layout::rect_of(&rects, i as u32).expect("every item is shown"))
}

/// The preset button's width in U for a name `text` wide: the text width / U + 1, at most 5. `text` and `u`
/// in the same unit.
fn preset_width(text: f32, u: f32) -> f32 {
    (text / u + 1.0).min(5.0)
}

/// The preset button's ▶: corners (R − 0.35 U, cy ∓ 0.175 U) and (R − 0.175 U, cy), R the right edge of
/// `rect`, cy its centre line. `rect` and `u` in the same unit.
fn play_triangle(rect: Rect, u: f32) -> [Pos2; 3] {
    let (r, cy) = (rect.right(), rect.center().y);
    [
        pos2(r - 0.35 * u, cy - 0.175 * u),
        pos2(r - 0.35 * u, cy + 0.175 * u),
        pos2(r - 0.175 * u, cy),
    ]
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x
}

/// The preset button: the name left-aligned 0.2 U in, 10 pt, truncated with an end ellipsis, and a ▶ at the right. The
/// name ends 0.5 U in, 0.15 U short of the ▶, so a cut name's ellipsis stays clear of it; a name the button is sized
/// for (text + 1 U) still fits.
fn preset_button(ui: &Ui, cx: &UiCtx, rect: Rect, name: &str, color: Color32) {
    let m = cx.m;
    let text = Rect::from_x_y_ranges(
        rect.left() + m.u(0.2)..=rect.right() - m.u(0.5),
        rect.y_range(),
    );
    let font = m.font_id(FontSize::Pt10);
    widgets::truncated(
        ui.painter(),
        text,
        Align2::LEFT_CENTER,
        name,
        font,
        color,
        0,
    );
    let tip = play_triangle(rect, m.u(1.0)).to_vec();
    ui.painter()
        .add(Shape::convex_polygon(tip, color, Stroke::NONE));
}

/// The module settings dialog's title in its row: 12 pt, centred, its bottom on the row's, truncated keeping 5 tail
/// characters.
fn dialog_title(painter: &Painter, cx: &UiCtx, row: Rect, title: &str, color: Color32) {
    let font = cx.m.font_id(FontSize::Pt12);
    widgets::truncated(painter, row, Align2::CENTER_BOTTOM, title, font, color, 5);
}

// The input row's imports, kept next to its code.
use crate::recording::input::{InputChoice, InputDevice, input_menu};
use crate::recording::routing::Route;

/// The level meter's rings around the mic button: one per 25 ms peak of the input's history.
pub const METER_RINGS: usize = 48;
/// The input row fades in over this many frames (its opacity, so the cards below keep their places).
pub const INPUT_APPEAR_FRAMES: u32 = 12;
/// The meter's alpha while monitoring, and otherwise (faint: that alpha at the 30 % disabled alpha).
const METER_ALPHA: f32 = 0.5;
const METER_FAINT: f32 = 0.15;
/// How often a shown meter asks for a frame: once per peak window.
const METER_REPAINT: std::time::Duration = std::time::Duration::from_millis(25);

/// The level meter's alpha of ring `k` (0 the newest 25 ms window) at its window's peak `peak` (linear): f^(5/3)·(1 −
/// k/48)², f = (20·log10(peak) + 60)/66, held to [0, 1] (−60 dB and below draws nothing, +6 dB is whole).
pub fn meter_alpha(peak: f32, k: usize) -> f32 {
    let f = (20.0 * peak.log10() + 60.0) / 66.0;
    // NaN (no peak) is nothing too.
    let f = if f > 0.0 { f.min(1.0) } else { 0.0 };
    f.powf(5.0 / 3.0) * (1.0 - k as f32 / METER_RINGS as f32).powi(2)
}

/// The radius of ring `k` around a mic button of radius `r` in a rack `rack_width` wide: r + (k/48)·0.4·rack width.
pub fn meter_radius(r: f32, k: usize, rack_width: f32) -> f32 {
    r + k as f32 / METER_RINGS as f32 * 0.4 * rack_width
}

/// The input row's opacity on the `frames`-th frame it is shown (from 1): frames/12, whole from the 12th on.
pub fn input_appear(frames: u32) -> f32 {
    frames.min(INPUT_APPEAR_FRAMES) as f32 / INPUT_APPEAR_FRAMES as f32
}

/// The `INPUT` menu ([`input_menu`]) over `devices`, with `current`, the channel's input, marked.
pub fn input_items(
    devices: &[InputDevice],
    current: Option<InputChoice>,
) -> Vec<PopupItem<Option<InputChoice>>> {
    let mut items = input_menu(devices);
    for item in &mut items {
        item.current = current.is_some() && item.id == Some(current);
    }
    items
}

/// What the audio-input row draws: its rect and the rack's width (points), the channel colour and the contrast colour,
/// the channel's CHHD monitor and arm flags, the multitrack setting and the input's peaks.
struct InputRow {
    row: Rect,
    rack_width: f32,
    ink: Color32,
    contrast: Color32,
    monitoring: bool,
    armed: bool,
    multitrack: bool,
    peaks: Option<[f32; METER_RINGS]>,
}

/// What was clicked in the audio-input row.
#[derive(Default)]
struct InputClicks {
    monitor: bool,
    arm: bool,
    /// `INPUT`: the menu opens here.
    menu: Option<Pos2>,
}

impl RackPanel {
    /// The audio-input row of `channel` ([`input_row`]), faded in over its first frames ([`input_appear`]): the mic
    /// button toggles its input monitoring (CHHD@1056) and the arm button its record arm (CHHD@1060), each a final song
    /// edit (one undo step, a new engine); `INPUT` opens the menu and asks the host for the devices.
    fn input(
        &mut self,
        ui: &mut Ui,
        cx: &UiCtx,
        row: &InputRow,
        channel: &mut Channel,
        actions: &mut RackActions,
    ) {
        let ctx = ui.ctx().clone();
        let frame = ctx.cumulative_frame_nr();
        if frame > self.shown_frame + 1 {
            self.input_frames = 0;
        }
        self.shown_frame = frame;
        self.input_frames = self.input_frames.saturating_add(1);
        let appear = input_appear(self.input_frames);
        if appear < 1.0 {
            ctx.request_repaint();
        }
        if row.peaks.is_some() {
            ctx.request_repaint_after(METER_REPAINT);
        }
        let opacity = ui.opacity();
        ui.multiply_opacity(appear);
        let clicks = input_row(ui, cx, row);
        ui.set_opacity(opacity);
        let file = &mut channel.file;
        if clicks.monitor {
            file.input_monitoring = !file.input_monitoring;
        }
        if clicks.arm {
            file.record_armed = !file.record_armed;
        }
        actions.replace_song |= clicks.monitor || clicks.arm;
        if let Some(at) = clicks.menu {
            (self.input_menu, self.inputs) = (Some(at), None);
            actions.list_inputs = true;
        }
    }
}

/// The audio-input row in `r.row`:
/// - the mic button, a 2 U icon button (icon 22) centred in the upper half: the icon buttons' light disc with a
///   `moduleFill` icon, or while monitoring a channel-colour disc with a contrast-colour icon;
/// - around it the level meter, clipped to the row: [`METER_RINGS`] concentric filled circles in the channel colour,
///   the oldest outermost, at [`meter_radius`] and [`meter_alpha`] times 0.5 while monitoring, faint otherwise;
/// - with multitrack on, the arm button (icon 5, 2 U, the mic button's look) at the upper half's right end;
/// - `INPUT` (12 pt, `text`) centred in the lower half, which opens the `INPUT` menu.
fn input_row(ui: &mut Ui, cx: &UiCtx, r: &InputRow) -> InputClicks {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (top, bottom) = r.row.split_top_bottom_at_fraction(0.5);
    let size = Vec2::splat(m.u(INPUT_BUTTON));
    let mic = Rect::from_center_size(top.center(), size);
    if let Some(peaks) = &r.peaks {
        let base = if r.monitoring {
            METER_ALPHA
        } else {
            METER_FAINT
        };
        let painter = ui.painter().with_clip_rect(r.row);
        let radius = ICON_DISC * 0.5 * mic.height();
        for (k, &peak) in peaks.iter().enumerate().rev() {
            let alpha = base * meter_alpha(peak, k);
            if alpha > 0.0 {
                let ring = meter_radius(radius, k, r.rack_width);
                painter.circle_filled(mic.center(), ring, r.ink.gamma_multiply(alpha));
            }
        }
    }
    let mut toggle = |id: &str, rect: Rect, icon: IconId, on: bool| {
        let b = Button {
            icon: Some(icon),
            shape: ICON_DISC,
            fill_on: r.ink,
            text_on: r.contrast,
            fill_off: colors.text.to_color32(),
            text_off: Some(colors.module_fill.to_color32()),
            ..Button::new(ui.id().with(id), rect, "", true, colors)
        };
        widgets::button(ui, cx, &b, on).clicked()
    };
    let monitor = toggle("record", mic, IconId::AudioRecord, r.monitoring);
    let arm = r.multitrack && {
        let arm = Rect::from_min_size(pos2(top.right() - size.x, top.top()), size);
        toggle("arm", arm, IconId::Record, r.armed)
    };
    let text = colors.text.to_color32();
    widgets::text_label(
        ui,
        cx,
        bottom,
        "INPUT",
        FontSize::Pt12,
        Align2::CENTER_CENTER,
        text,
    );
    let menu = ui.interact(bottom, ui.id().with("input"), Sense::click());
    InputClicks {
        monitor,
        arm,
        menu: menu.clicked().then(|| bottom.center()),
    }
}

/// Draws an `Add Module` / `Add Send` row, whose whole rect is the button: a centred group of a circle
/// (radius 0.4 × the row height), a 0.5 U gap and the text (10 pt) in the channel colour `ink`. With `hole`, the
/// circle is filled and the plus cut out of it in that colour; without, it is a 2 px ring with a plus in `ink`.
fn add_row(ui: &Ui, cx: &UiCtx, row: Rect, text: &str, ink: Color32, hole: Option<Color32>) {
    let m = cx.m;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text.to_owned(), m.font_id(FontSize::Pt10), ink);
    let r = 0.4 * row.height();
    let left = row.center().x - (2.0 * r + m.u(0.5) + galley.size().x) / 2.0;
    let c = pos2(left + r, row.center().y);
    let plus = match hole {
        Some(hole) => {
            painter.circle_filled(c, r, ink);
            hole
        }
        None => {
            let ring = 2.0 / m.ppp();
            painter.circle_stroke(c, r - ring / 2.0, Stroke::new(ring, ink));
            ink
        }
    };
    let (len, t) = (1.2 * r, m.t());
    painter.rect_filled(Rect::from_center_size(c, vec2(len, t)), 0.0, plus);
    painter.rect_filled(Rect::from_center_size(c, vec2(t, len)), 0.0, plus);
    let text_at = pos2(left + 2.0 * r + m.u(0.5), c.y - galley.size().y / 2.0);
    painter.galley(text_at, galley, ink);
}

/// A send row in `row`: its 1 px `text` outline, and the rects of its pill toggle (1.5 U), swap icon (2 U),
/// target name and level slider (flex 1 each) and remove icon (2 U).
fn send_row(ui: &Ui, cx: &UiCtx, row: Rect) -> [Rect; 5] {
    let (m, text) = (cx.m, cx.theme.colors.text.to_color32());
    let ppp = m.ppp();
    ui.painter()
        .rect_stroke(row, 0.0, Stroke::new(1.0 / ppp, text), StrokeKind::Inside);
    let items = Node::hstack([
        Node::leaf(0).w(Size::U(1.5)),
        Node::leaf(1).w(Size::U(2.0)),
        Node::leaf(2),
        Node::leaf(3),
        Node::leaf(4).w(Size::U(2.0)),
    ]);
    let rects = layout::layout(&items, row * ppp, m.u_px());
    std::array::from_fn(|i| layout::rect_of(&rects, i as u32).expect("shown") / ppp)
}

/// A send row's icon button: `icon` in the channel colour `ink`, no disc. Whether it was clicked.
fn icon_button(ui: &mut Ui, cx: &UiCtx, id: Id, rect: Rect, icon: IconId, ink: Color32) -> bool {
    draw_icon(ui.painter(), rect, icon, ink, SEND_ICON, cx.m);
    ui.interact(rect, id, Sense::click()).clicked()
}

/// Where the Mix / Post Gain / Wet Pan dialog goes, in points: 10 U × 8 U with its top-left corner at the icon centre
/// `at`, moved up as far as its bottom would leave `screen`, on whole pixels.
pub fn level_popup_rect(at: Pos2, screen: Rect, m: &Metrics) -> Rect {
    let size = DIALOG * m.u(1.0);
    let min = pos2(
        m.round_px(at.x),
        m.round_px(at.y.min(screen.bottom() - size.y)),
    );
    Rect::from_min_size(min, size)
}

/// The Mix / Post Gain / Wet Pan dialog: 10 U × 8 U with its top-left corner at `at`, moved up to stay on screen. Inset
/// 0.25 U, rows weighted 1 : 5 : 5 : 5: the module name (12 pt), then the `Wet Pan`, `Mix` and `Post Gain` sliders with
/// value text, all in `text`. The fill is `menuFill`. Modal like the popups: a click outside it or Escape closes it,
/// drawing no dialog (and on Escape no backdrop) in that frame. Returns the common params changed, whether to close and
/// a slider's touch.
fn common_dialog(
    ui: &Ui,
    cx: &UiCtx,
    id: Id,
    at: Pos2,
    module: &mut ModuleInstance,
) -> (Vec<i32>, bool, Option<Touch>) {
    let (m, colors) = (cx.m, &cx.theme.colors);
    let (ppp, u) = (m.ppp(), m.u_px());
    let ctx = ui.ctx();
    let screen = ctx.content_rect();
    let rect = level_popup_rect(at, screen, m);

    // Closing draws nothing. Escape returns before the backdrop: egui hit-tests a frame against the last one's
    // widgets, so a backdrop drawn now would take a press in the next frame.
    if crate::ui::close_pressed(ctx) {
        return (Vec::new(), true, None);
    }
    // Below the dialog, above the screens: it takes every click outside the dialog.
    let backdrop = crate::ui::backdrop(ctx, id);
    if backdrop.clicked() || backdrop.secondary_clicked() {
        return (Vec::new(), true, None);
    }

    let instance_id = module.instance_id;
    let (changed, touch) = Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            // As large as the dialog, so it swallows the clicks between the sliders.
            ui.allocate_exact_size(rect.size(), Sense::hover());
            ui.painter()
                .rect_filled(rect, 0.0, colors.menu_fill.to_color32());
            let text = colors.text.to_color32();
            let column = Node::vstack(
                [1.0, 5.0, 5.0, 5.0]
                    .into_iter()
                    .zip(0..)
                    .map(|(w, i)| Node::leaf(i).h(Size::Flex(w))),
            );
            let content = (rect * ppp).shrink((DIALOG_INSET * u).round());
            let rows = layout::layout(&column, content, u);
            let row = |i: u32| layout::rect_of(&rows, i).expect("shown") / ppp;
            // The 12 pt title is taller than its row; it may spill over the dialog's padding, not out of it.
            let title = module_name(module.type_id);
            dialog_title(&ui.painter_at(rect), cx, row(0), title, text);
            let (mut changed, mut touch) = (Vec::new(), None);
            let linked = panels::cc_links(ui);
            for (&(index, spec), i) in COMMON.iter().zip(1..) {
                let value = module.param_mut(index).expect("a common param");
                let c = Control {
                    value_text: true,
                    menu: ParamMenu::bound(links::has(&linked, instance_id, index)),
                    fine_tune: crate::ui::fine_tune::module_param(&spec),
                    ..Control::new(id.with(index), row(i), spec.name, text, spec.default)
                };
                if widgets::h_slider(ui, cx, &c, value, |v| spec.display_text(v)).changed() {
                    changed.push(index);
                }
                let target = AutomationTarget {
                    instance_id,
                    param: index,
                };
                touch = Touch::new(target, widgets::signal(ui, c.id)).or(touch);
            }
            (changed, touch)
        })
        .inner;
    (changed, false, touch)
}
