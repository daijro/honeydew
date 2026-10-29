//! The audio clip editor's sample row. It sits in the keyboard area instead of the keyboard while the current
//! channel's main lane is audio and a clip is current ([`current`]), and edits that clip's sampler slot:
//! - a sample-picker icon button (icon 21), which opens the sample picker for the slot;
//! - the sliders `Level`, `Pan` and `Speed` (slot `PRMS` 0–2, with their value laws) and `Pitch` (`STR1` semitones,
//!   normalised as `STRC`'s st/48 + 0.5);
//! - the text toggles `REV` (`MAIN`@1036) and `SYNC` (`STR1` mode; the ratio stays as stored, for a later SYNC-on).
//!
//! The row draws from a [`Row`] and returns a [`RowAction`]; the app writes it ([`apply`]) as a song edit, ongoing
//! while a slider drags and final (one undo step and an engine rebuild, which re-renders the pitch) on its release or
//! a toggle's click.

use eframe::egui::{Color32, Id, Rect, Ui, pos2, vec2};

use super::UiCtx;
use super::keyboard_settings::pads_rect;
use super::preview::ClipKey;
use super::views::audio_editor;
use super::widgets::{self, Button, Control, ParamMenu};
use crate::icons::IconId;
use crate::metrics::{FontSize, Metrics};
use crate::params::{DRUM_SLOT_PARAMS, Fmt};
use crate::song::{Clip, LaneKind, SamplerSlot, Song};
use honeydew_audio::sample_edit::speed_semitones;

/// The row's size, U: 30 wide (at most the area's width) and 2 high.
const ROW_W: f32 = 30.0;
const ROW_H: f32 = 2.0;
/// The icon button and each text toggle, U wide.
const CELL_W: f32 = 2.0;
/// The icon button's disc, of the cell: the drum strip's sample button (`drums.rs`).
const ICON_SHAPE: f32 = 0.7;
/// Pitch steps: −24 … +24 semitones.
const PITCH_STEPS: usize = 49;

/// The row's controls, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ctl {
    Pick,
    Level,
    Pan,
    Speed,
    Pitch,
    Rev,
    Sync,
}

const CTLS: [Ctl; 7] = [
    Ctl::Pick,
    Ctl::Level,
    Ctl::Pan,
    Ctl::Speed,
    Ctl::Pitch,
    Ctl::Rev,
    Ctl::Sync,
];

/// The id of the row's control `ctl`.
pub fn id(ctl: Ctl) -> Id {
    Id::new("sample_row").with(ctl)
}

/// What the row shows: the slot's values, the sliders' normalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub level: f32,
    pub pan: f32,
    pub speed: f32,
    /// `STR1` pitch as st/48 + 0.5.
    pub pitch: f32,
    pub reverse: bool,
    pub sync: bool,
}

impl Row {
    /// `slot`'s values; a clip without a slot shows the default slot's.
    pub fn of(slot: Option<&SamplerSlot>) -> Self {
        let default = SamplerSlot::default();
        let slot = slot.unwrap_or(&default);
        let [level, pan, speed, ..] = slot.params;
        Self {
            level,
            pan,
            speed,
            // A file may hold any pitch: the slider shows it at its end.
            pitch: (slot.file.pitch_semitones / 48.0 + 0.5).clamp(0.0, 1.0) as f32,
            reverse: slot.file.reverse,
            sync: slot.file.stretch_mode != 0,
        }
    }

    /// Control `ctl`'s value (the icon button has none).
    fn get(self, ctl: Ctl) -> f32 {
        match ctl {
            Ctl::Pick => 0.0,
            Ctl::Level => self.level,
            Ctl::Pan => self.pan,
            Ctl::Speed => self.speed,
            Ctl::Pitch => self.pitch,
            Ctl::Rev => self.reverse.into(),
            Ctl::Sync => self.sync.into(),
        }
    }
}

/// What the user did on the row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowAction {
    /// The icon button: open the sample picker for the slot.
    Pick,
    /// Control `ctl` is now `v`; `done`: the gesture ended (a slider's release, a toggle's click, a reset).
    Set { ctl: Ctl, v: f32, done: bool },
}

/// The clip the row edits on `channel`: none unless the channel's main lane is audio and shows (not hidden, `Hide track`,
/// or `show_hidden`: Settings `Show hidden tracks`); then the audio editor's clip for the channel (`editor`), else the
/// main lane's current clip (`DESc`@24).
pub fn current(
    song: &Song,
    channel: usize,
    editor: &audio_editor::State,
    show_hidden: bool,
) -> Option<ClipKey> {
    let main = song.channels.get(channel)?.lanes.first()?;
    if main.kind != LaneKind::Audio || main.hidden && !show_hidden {
        return None;
    }
    editor.current(song, channel).or_else(|| {
        let id = main.file.current_clip_id;
        let key = ClipKey {
            channel,
            lane: 0,
            id,
        };
        main.clips.iter().any(|c| c.id == id).then_some(key)
    })
}

/// The clip `key` names.
pub fn clip_mut(song: &mut Song, key: ClipKey) -> Option<&mut Clip> {
    let lane = song
        .channels
        .get_mut(key.channel)?
        .lanes
        .get_mut(key.lane)?;
    lane.clips.iter_mut().find(|c| c.id == key.id)
}

/// Writes control `ctl`'s value `v` into `slot`.
pub fn set(slot: &mut SamplerSlot, ctl: Ctl, v: f32) {
    match ctl {
        Ctl::Pick => {}
        Ctl::Level => slot.params[0] = v,
        Ctl::Pan => slot.params[1] = v,
        Ctl::Speed => slot.params[2] = v,
        Ctl::Pitch => slot.file.pitch_semitones = semitones(v).round().into(),
        Ctl::Rev => slot.file.reverse = v >= 0.5,
        Ctl::Sync => slot.file.stretch_mode = u8::from(v >= 0.5),
    }
}

/// Writes control `ctl`'s value `v` into the slot of `key`'s clip, giving a clip without one the default slot. Returns
/// whether the song changed.
pub fn apply(song: &mut Song, key: ClipKey, ctl: Ctl, v: f32) -> bool {
    let Some(clip) = clip_mut(song, key) else {
        return false;
    };
    let mut slot = clip.file.sample.as_deref().cloned().unwrap_or_default();
    set(&mut slot, ctl, v);
    let changed = clip.file.sample.as_deref() != Some(&slot);
    if changed {
        clip.file.sample = Some(Box::new(slot));
    }
    changed
}

/// The controls' rects in [`CTLS`] order, for the keys area `keys`: in its pads rect ([`pads_rect`], 0.35 U lower), the
/// row is the width intersected with a centred [`ROW_W`] U span, and [`ROW_H`] U tall in a column of [flex, row, flex]
/// (centred); under [`ROW_H`] U the flex spaces are 0 and the row sits at the top, overflowing below. It holds the icon
/// button, the four sliders sharing what is left equally, and the two toggles. Edges sit on whole device pixels.
pub fn layout(keys: Rect, m: &Metrics) -> [Rect; 7] {
    let area = pads_rect(keys, m);
    let top = area.top().max(area.center().y - m.u(ROW_H) / 2.0);
    let row = Rect::from_min_size(
        pos2(area.center().x - m.u(ROW_W).min(area.width()) / 2.0, top),
        vec2(m.u(ROW_W).min(area.width()), m.u(ROW_H)),
    );
    let (top, cell) = (m.round_px(row.top()), m.u(CELL_W));
    let slider = (row.width() - 3.0 * cell) / 4.0;
    let mut x = row.left();
    CTLS.map(|ctl| {
        let w = match ctl {
            Ctl::Pick | Ctl::Rev | Ctl::Sync => cell,
            _ => slider,
        };
        let left = m.round_px(x);
        x += w;
        Rect::from_min_max(pos2(left, top), pos2(m.round_px(x), top + m.u(ROW_H)))
    })
}

/// `48v − 24` in f32, Pitch's semitone law (Speed's is the engine's [`speed_semitones`]).
fn semitones(v: f32) -> f32 {
    48.0f32.mul_add(v, -24.0)
}

/// `semitone` or `semitones`.
fn plural(n: i32) -> &'static str {
    if n.abs() == 1 { "" } else { "s" }
}

/// `Level: %.1f dB` (dB = 66·v^0.6 − 60), `Level: -inf.` at v = 0.
pub fn level_text(v: f32) -> String {
    if v <= 0.0 {
        return "Level: -inf.".to_owned();
    }
    let db = 66.0f32.mul_add(v.powf(0.6), -60.0);
    format!("Level: {}", Fmt::new(1, " dB").apply(db.into()))
}

/// `Pan: center` or `Pan: %d%% left|right`, as the slot's Pan reads.
pub fn pan_text(v: f32) -> String {
    format!("Pan: {}", DRUM_SLOT_PARAMS[1].display_text(v))
}

/// `Speed: 1.0x` or `Speed: %+d semitone%s`, trunc(48v − 24) as the engine plays it ([`speed_semitones`]).
pub fn speed_text(v: f32) -> String {
    match speed_semitones(v) {
        0 => "Speed: 1.0x".to_owned(),
        n => format!("Speed: {n:+} semitone{}", plural(n)),
    }
}

/// `Pitch: %s%d semitone%s`, lround(48v − 24): `+` on raised pitches.
pub fn pitch_text(v: f32) -> String {
    let n = semitones(v).round() as i32;
    let sign = if n > 0 { "+" } else { "" };
    format!("Pitch: {sign}{n} semitone{}", plural(n))
}

/// The text a slider shows.
fn text(ctl: Ctl, v: f32) -> String {
    match ctl {
        Ctl::Level => level_text(v),
        Ctl::Pan => pan_text(v),
        Ctl::Speed => speed_text(v),
        _ => pitch_text(v),
    }
}

/// Draws the row in the keys area `area` ([`layout`]; nothing while the area is 0 tall), its slider texts
/// and fills in the channel colour `ink`, showing `row`.
///
/// The sliders put their text under the track (the value strip, so it follows a drag in its own frame), with the
/// slot's default for a double click or `Reset`; the toggles are text toggles in 10 pt; the icon button is the drum
/// strip's sample button.
pub fn show(ui: &mut Ui, cx: &UiCtx, area: Rect, ink: Color32, row: Row) -> Option<RowAction> {
    if area.height() == 0.0 {
        return None;
    }
    let colors = &cx.theme.colors;
    let mut action = None;
    for (ctl, rect) in CTLS.into_iter().zip(layout(area, cx.m)) {
        let mut v = row.get(ctl);
        let set = |v, done| Some(RowAction::Set { ctl, v, done });
        match ctl {
            Ctl::Pick => {
                let b = Button {
                    icon: Some(IconId::AudioClip),
                    shape: ICON_SHAPE,
                    fill_on: colors.text.to_color32(),
                    text_on: colors.module_fill.to_color32(),
                    ..Button::new(id(ctl), rect, "", true, colors)
                };
                if widgets::button(ui, cx, &b, true).clicked() {
                    action = Some(RowAction::Pick);
                }
                crate::ui::tooltip::tip(ui, rect, "Load sample", None);
            }
            Ctl::Rev | Ctl::Sync => {
                let label = if ctl == Ctl::Rev { "REV" } else { "SYNC" };
                let c = Control {
                    label_font: FontSize::Pt10,
                    menu: ParamMenu::Off,
                    ..Control::new(id(ctl), rect, label, ink, 0.0)
                };
                if widgets::text_toggle(ui, cx, &c, &mut v).changed() {
                    action = set(v, true);
                }
            }
            _ => {
                let default = Row::of(None).get(ctl);
                let c = Control {
                    steps: (ctl == Ctl::Pitch).then_some(PITCH_STEPS),
                    value_text: true,
                    menu: ParamMenu::Unbound,
                    ..Control::new(id(ctl), rect, "", ink, default)
                };
                let r = widgets::h_slider(ui, cx, &c, &mut v, |v| text(ctl, v));
                if r.changed() || r.drag_stopped() {
                    action = set(v, !r.dragged());
                }
            }
        }
    }
    action
}

#[cfg(test)]
mod tests;
