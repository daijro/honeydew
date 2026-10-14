//! The fine tune dialog, opened by `Fine tune control`: the CTRL menu's sixth row, a parameter menu's fifth.
//!
//! - **Availability** ([`supported`]): channel Level, Pan and Send Level; drum pad params 0…10 but Mute (3) and
//!   Solo (4); module params that are continuous, with no discrete step count. Never a stepped or two-state param.
//! - **Box** ([`layout`]): a dialog box 8 U high, as wide as the screen and centred on it vertically, with a 2 U close
//!   button at its top right (the close icon at 0.75 scale). Its content is inset 2 U left, right and top and is 5 U
//!   high: three rows of a third each holding the title `<channel name> | <param scope name> | <param name>`, the fine
//!   tune slider ([`widgets::fine_tune_slider`]) and the param's own value text, both texts FontSize 4 (18 pt) in
//!   `text`, centred. Nothing else: no `Default:`, no field, no OK.
//! - **Behaviour** ([`show`]): the slider edits the value v (0…1) itself; its reset value, for the double tap, is the
//!   param default ([`Param::reset`]). The close button, Escape or a tap outside the box closes it (the tap outside as
//!   in the channel settings dialog). A dialog box: no animation, the screen dimmed to α 0.6 under a `moduleFill` box
//!   from the first frame.
//!
//! The app's slot ([`crate::ui::views::fine_tune`]) writes each change to the param as a hand move does.

use eframe::egui::{Align2, Area, Color32, Context, Id, Order, Rect, Sense, Vec2, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::{FontSize, Metrics};
use crate::params::{
    CHANNEL_PARAMS, COMMON_PARAMS, DRUM_SLOT_PARAMS, ParamKind, ParamSpec, Role, module_spec,
};
use crate::song::{AutomationTarget, Song};
use crate::ui::widgets::{self, Control, ParamMenu};
use crate::ui::{UiCtx, dialog};

/// The box's height, in U.
const HEIGHT: f32 = 8.0;
/// The close button's edge, in U, and its icon's scale.
const CLOSE: f32 = 2.0;
const CLOSE_ICON: f32 = 0.75;
/// The content's inset from the box's left, right and top, and its height, in U.
const INSET: f32 = 2.0;
const CONTENT_H: f32 = 5.0;
/// The title's and the value text's size: FontSize 4.
const FONT: FontSize = FontSize::Pt18;

/// The reset values of drum pad params 0…10 (Mute and Solo are never fine-tuned).
pub const PAD_RESETS: [f32; 11] = [0.85312, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5];

/// Whether the dialog supports `target`'s param, whose registry spec is `spec` (only a module param's is read).
pub fn supported(target: AutomationTarget, spec: Option<&ParamSpec>) -> bool {
    match target.instance_id {
        0 => channel_param(target.param),
        id if id < 0 => pad_param(target.param),
        _ => spec.is_some_and(module_param),
    }
}

/// A channel param (instance 0) the dialog supports: Level (0), Pan (1) and the send levels (0x2000 + a channel id).
pub fn channel_param(param: i32) -> bool {
    matches!(param, 0 | 1 | 0x2000..=0x2FFF)
}

/// A drum pad param the dialog supports: 0…10 but Mute (3) and Solo (4).
pub fn pad_param(param: i32) -> bool {
    matches!(param, 0..=2 | 5..=10)
}

/// A module param, the common ones among them, the dialog supports: continuous, with no step count
/// ([`crate::params::Display::step_count`]).
pub fn module_param(spec: &ParamSpec) -> bool {
    matches!(spec.kind, ParamKind::Continuous { display } if display.step_count().is_none())
}

/// What the dialog shows of a param.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// `<channel name> | <param scope name> | <param name>`. The scope is the module's display name, the pad's
    /// name for a drum pad param, `Channel` for a channel param. The name is the registry's automation name, a send
    /// level's `Send Level (to <channel>)`.
    pub title: String,
    /// The registry's spec, for the value text; a send level reads as the channel Level.
    pub spec: ParamSpec,
    /// The slider's reset value, the param default: the module's ([`ParamSpec::default`]), [`PAD_RESETS`], or unity
    /// ([`widgets::UNITY`]) for the channel Level and 0.5 for its Pan (a send level resets to unity, as its rack
    /// slider does).
    pub reset: f32,
    /// A note effect's param: its edit rebuilds the engine at the end of the drag, as a rack card's does.
    pub note_effect: bool,
}

/// The dialog's view of `target`'s param on channel `channel`, `None` when the param is gone or the dialog does not
/// support it ([`supported`]).
pub fn param(song: &Song, channel: usize, target: AutomationTarget) -> Option<Param> {
    let ch = song.channels.get(channel)?;
    ch.param(target)?;
    let index = usize::try_from(target.param);
    let (scope, spec, reset, note_effect) = match target.instance_id {
        0 => {
            let (spec, reset) = match target.param {
                0x2000..=0x2FFF => (CHANNEL_PARAMS[0], widgets::UNITY),
                1 => (CHANNEL_PARAMS[1], 0.5),
                _ => (*CHANNEL_PARAMS.get(index.ok()?)?, widgets::UNITY),
            };
            ("Channel", spec, reset, false)
        }
        id if id < 0 => {
            let slot = ch.drum_kit.as_ref()?.slot_by_id(id.unsigned_abs())?;
            // As its automation lanes name it: the pad's name, else its sample's.
            let pad = [slot.display_name(), &slot.name]
                .into_iter()
                .find(|n| !n.is_empty());
            let i = index.ok()?;
            (
                pad.unwrap_or_default(),
                *DRUM_SLOT_PARAMS.get(i)?,
                *PAD_RESETS.get(i)?,
                false,
            )
        }
        id => {
            let module = module_spec(ch.rack.get(id)?.type_id)?;
            let spec = match index {
                Ok(i) => *module.params.get(i)?,
                // −1, −2, −3: Post Gain, Mix, Wet Pan.
                Err(_) => *COMMON_PARAMS.get(usize::try_from(-1 - target.param).ok()?)?,
            };
            let note = module.role == Role::NoteEffect;
            (module.display_name, spec, spec.default, note)
        }
    };
    if !supported(target, Some(&spec)) {
        return None;
    }
    let name = match target.instance_id {
        0 => song.automation_label(channel, target)?,
        _ => spec.name.to_owned(),
    };
    Some(Param {
        title: format!("{} | {scope} | {name}", ch.name),
        spec,
        reset,
        note_effect,
    })
}

/// Where the dialog's parts go, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub dialog: Rect,
    pub close: Rect,
    /// The title, the slider and the value text, top to bottom: thirds of the content, on whole device pixels.
    pub rows: [Rect; 3],
}

/// The dialog's layout on `screen`: 8 U high, its full width, centred vertically (on a whole device pixel).
pub fn layout(screen: Rect, m: &Metrics) -> Layout {
    let u = m.u(1.0);
    let top = m.round_px(screen.center().y - HEIGHT / 2.0 * u);
    let dialog = Rect::from_min_size(pos2(screen.left(), top), vec2(screen.width(), HEIGHT * u));
    let close = Rect::from_min_size(
        pos2(dialog.right() - CLOSE * u, dialog.top()),
        Vec2::splat(CLOSE * u),
    );
    let content = Rect::from_x_y_ranges(
        dialog.left() + INSET * u..=dialog.right() - INSET * u,
        dialog.top() + INSET * u..=dialog.top() + (INSET + CONTENT_H) * u,
    );
    let y = |i: usize| m.round_px(content.top() + i as f32 * content.height() / 3.0);
    let rows = [0, 1, 2].map(|i| Rect::from_x_y_ranges(content.x_range(), y(i)..=y(i + 1)));
    Layout {
        dialog,
        close,
        rows,
    }
}

/// What the dialog shows besides the value.
#[derive(Clone, Copy, Debug)]
pub struct Dialog<'a> {
    /// [`Param::title`].
    pub title: &'a str,
    /// The channel colour: the slider's.
    pub ink: Color32,
    /// [`Param::reset`].
    pub reset: f32,
}

/// What one frame of the dialog did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// The slider changed the value.
    pub changed: bool,
    /// The close button, Escape or a tap outside the box: the dialog closes.
    pub close: bool,
}

/// The dialog's id; its slider's is `id().with("slider")`.
pub fn id() -> Id {
    Id::new("fine tune")
}

/// Shows the dialog on `screen`, modal, editing `value` with `text` its value text.
pub fn show(
    ctx: &Context,
    cx: &UiCtx,
    screen: Rect,
    d: &Dialog,
    value: &mut f32,
    text: impl Fn(f32) -> String,
) -> Shown {
    let (m, c) = (cx.m, &cx.theme.colors);
    let ink = c.text.to_color32();
    let l = layout(screen, m);
    let id = id();
    // A dialog box: the dim and the box show from the first frame ([`dialog::paint_in_layers`]).
    dialog::paint_in_layers(ctx, id, c, screen, l.dialog);
    // Below the box, above the screens: it takes every touch outside the box. Not [`crate::ui::backdrop`], which marks
    // the pass modal and so holds Ctrl+Z: undo acts under this dialog, which then shows the song's value. It is the one
    // dialog on the base that is not modal for undo.
    let backdrop = Area::new(id.with("backdrop"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .constrain(false)
        .fade_in(false)
        .sense(Sense::click_and_drag())
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::hover())
        })
        .response;
    let escape = crate::ui::close_pressed(ctx);
    let mut shown = Shown {
        changed: false,
        close: escape || backdrop.clicked() || backdrop.secondary_clicked(),
    };
    Area::new(id)
        .order(Order::Foreground)
        .fixed_pos(l.dialog.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.allocate_exact_size(l.dialog.size(), Sense::hover());
            let font = m.font_id(FONT);
            let [title, slider, value_row] = l.rows;
            let center = Align2::CENTER_CENTER;
            ui.painter()
                .text(title.center(), center, d.title, font.clone(), ink);
            // The dialog shows no other text: the slider has no menu of its own.
            let control = Control {
                menu: ParamMenu::Off,
                ..Control::new(id.with("slider"), slider, "", d.ink, d.reset)
            };
            shown.changed = widgets::fine_tune_slider(ui, cx, &control, value).changed();
            ui.painter()
                .text(value_row.center(), center, text(*value), font, ink);
            draw_icon(ui.painter(), l.close, IconId::Close, ink, CLOSE_ICON, m);
            shown.close |= ui
                .interact(l.close, id.with("close"), Sense::click())
                .clicked();
        });
    shown
}

#[cfg(test)]
mod tests;
