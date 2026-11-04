//! A built-in Lemon plugin's card ([`crate::plugins::builtin`]). Below the header the card shows the plugin's own
//! editor, drawn by the plugin's own code (`os_plugkit::embed`) into a texture at the card's width, its aspect kept,
//! the same on every platform.
//!
//! - The pointer over it is the editor's, in the editor's pixels: presses (a second one within 0.4 s and 4 px is a
//!   double click), moves, releases (also outside, while a press it took is held) and the wheel. While the editor holds
//!   the cursor (a knob drag) the moves go as movement and the cursor hides.
//! - The editor's menus show over the card in Lemon's own look (`os_ui::kit::menu`), with their submenus; its "Type in
//!   value" box is a text field there.
//! - Issue #52: where the editor has no file dialog (Android, the browser), the app's sample picker opens its sample
//!   ([`wants_file`]).
//! - Its edits reach the song as a plugin editor's do (`plugins::idle`): the song's params, undo, REC.

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Area, Color32, ColorImage, CursorIcon, Event, Id, Key, Modifiers, MouseWheelUnit, Order,
    PointerButton, Pos2, Rect, Sense, TextEdit, TextureHandle, TextureOptions, Ui, Vec2, pos2,
    vec2,
};
use os_plugkit::embed::{self, Answer, Button, Input, MenuView, Modal};
use os_ui::Canvas;
use os_ui::kit::menu::{self as kmenu, MenuLayout};

use crate::metrics::Metrics;
use crate::plugins;
use crate::plugins::builtin::Builtin;
use crate::ui::gesture;

/// Two presses closer than this (seconds and points) are a double click.
const DOUBLE_S: f64 = 0.4;
const DOUBLE_PT: f32 = 4.0;
/// The editor's polling period while it shows and is busy (its own timer on the desktop), and while it rests: busy is
/// within `BUSY_S` of a painted frame, or with the pointer on it. A parameter change wakes it at once.
// ponytail: a resting editor polls at 4 Hz (its overlays expire a little late); a wake from the editor itself would let
// it sleep.
const TICK: std::time::Duration = std::time::Duration::from_millis(30);
const REST: std::time::Duration = std::time::Duration::from_millis(250);
const BUSY_S: f64 = 0.5;
/// Points per wheel notch for a wheel that scrolls by points.
const WHEEL_POINTS: f32 = 50.0;
/// The type-in box's width in the editor's pixels at scale 1, as Lemon's.
const TYPE_IN_W: f32 = 140.0;

/// A Lemon card's own state: its editor's texture and the pointer's.
#[derive(Default)]
pub(super) struct LemonCard {
    /// The editor shown ([`plugins::lemon_editor`]).
    editor: embed::Parent,
    /// The editor's `Create automation clip` pick since the rack last read it: the parameter's list position
    /// ([`show`] sets it, the rack makes the parameter's automation track).
    pub(super) automate: Option<i32>,
    texture: Option<TextureHandle>,
    /// The frame shown, its size in pixels and when it came.
    version: u64,
    painted: f64,
    size: (u32, u32),
    /// The button of the press the editor took: its moves and release are the editor's wherever they are.
    pressed: Option<Button>,
    /// The press's long press was sent as a right click ([`forward`]); the left press was released with it, and the
    /// release of a press whose right click opened no modal pairs the right click with an up.
    long_menu: bool,
    /// The press left the calm zone before its long press came due: a drag, never a menu.
    long_cancel: bool,
    /// The pointer's last position, and whether it was over the editor.
    last: Pos2,
    over: bool,
    /// The last press: when and where, for a double click.
    click: Option<(f64, Pos2)>,
    menu: Option<MenuUi>,
    /// The type-in box's text, while it is open.
    typed: Option<String>,
    /// The editor asks for a file ([`Modal::OpenFile`]), and the request went to the app ([`wants_file`]) or is about to.
    asked_file: bool,
    wants_file: bool,
}

impl std::fmt::Debug for LemonCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LemonCard")
            .field("editor", &self.editor)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The height of a card body showing an editor of `size` at `width`.
pub(super) fn body_height(size: (u32, u32), width: f32) -> f32 {
    width * size.1 as f32 / size.0.max(1) as f32
}

/// Draws module `instance_id`'s editor (built-in plugin `plugin`, on the channel with id `channel_id`) in the card body
/// `content` (points). `false` while the plugin is not running: the caller shows the plugin card instead.
pub(super) fn show(
    ui: &mut Ui,
    content: Rect,
    channel_id: i32,
    instance_id: i32,
    plugin: &Builtin,
    card: &mut LemonCard,
    m: &Metrics,
) -> bool {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    // The editor at the card's width in device pixels, so that it is sharp ([`plugins::lemon_editor`] opens it again
    // only when that moves by more than a little).
    let scale = (content.width() * ppp / plugin.size.0 as f32).clamp(0.5, 4.0);
    // The handle's type is the platform's (`embed::Parent`: `usize` on macOS).
    let Some(editor) = plugins::lemon_editor(channel_id, instance_id, scale)
        .map(|id| -> embed::Parent { id as _ })
    else {
        return false;
    };
    if card.editor != editor {
        *card = LemonCard {
            editor,
            ..LemonCard::default()
        };
    }
    // The editor's "Create automation clip" picks (the Lemon editors share the link's queue; this card's are its own).
    if let Some(&index) = lemon_sampler::take_automation_requests(editor as u64).last() {
        card.automate = Some(index as i32);
    }
    let rect = Rect::from_min_size(
        content.min,
        vec2(content.width(), body_height(plugin.size, content.width())),
    );
    // Global (an editor shows in one card at a time): tests and render harnesses find the card by its editor.
    let id = Id::new(("lemon editor", editor));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    // Editor pixels per point.
    let k = card.size.0 as f32 / rect.width();
    let modal = embed::modal(editor);
    if modal.is_none() && card.size.0 > 0 {
        forward(ui, card, rect, k, response.hovered(), m);
    } else {
        // The press that asked is the modal's (its answer delivers it again, with its release): the next press is new.
        card.pressed = None;
        card.long_menu = false;
    }
    embed::pump(editor);
    let mut fresh = None;
    if let Some(v) = embed::frame(editor, card.version, |f| {
        fresh = Some(image(&f.pixels, f.width, f.height))
    }) {
        card.version = v;
        card.painted = ctx.input(|i| i.time);
    }
    if let Some(image) = fresh {
        card.size = (image.size[0] as u32, image.size[1] as u32);
        match &mut card.texture {
            Some(t) => t.set(image, TextureOptions::LINEAR),
            None => {
                card.texture = Some(ctx.load_texture(
                    format!("lemon editor {editor}"),
                    image,
                    TextureOptions::LINEAR,
                ))
            }
        }
    }
    if let Some(t) = &card.texture {
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, Color32::WHITE);
    }
    if card.pressed.is_some() && embed::locked(editor) {
        ctx.set_cursor_icon(CursorIcon::None);
    }
    match modal {
        Some(Modal::Menu { x, y, menu, scale }) => {
            show_menu(&ctx, id, card, rect, k, (x, y), menu, scale)
        }
        Some(Modal::TypeIn { x, y, text, scale }) => {
            type_in(&ctx, id, card, rect, k, (x, y), text, scale)
        }
        Some(Modal::OpenFile) => {
            card.wants_file |= !std::mem::replace(&mut card.asked_file, true);
            // A new press on the editor: the picker went without answering.
            if response.hovered() && ctx.input(|i| i.pointer.any_pressed()) {
                embed::answer(editor, None);
            }
        }
        None => (card.menu, card.typed, card.asked_file) = (None, None, false),
    }
    let now = ctx.input(|i| i.time);
    let busy =
        now - card.painted < BUSY_S || over(card) || card.menu.is_some() || card.typed.is_some();
    ctx.request_repaint_after(if busy { TICK } else { REST });
    true
}

/// Whether the editor asked for a file to open since the last call: the app's sample picker answers it.
pub(super) fn wants_file(card: &mut LemonCard) -> bool {
    std::mem::take(&mut card.wants_file)
}

/// Whether the pointer was over a Lemon editor this frame: the rack's wheel is then the editor's.
pub(super) fn over(card: &LemonCard) -> bool {
    card.over || card.pressed.is_some()
}

/// XRGB rows of `w` as an image.
fn image(px: &[u32], w: u32, h: u32) -> ColorImage {
    let pixels = (px.iter())
        .map(|&p| Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8))
        .collect();
    ColorImage::new([w as usize, h as usize], pixels)
}

fn button(b: PointerButton) -> Option<Button> {
    match b {
        PointerButton::Primary => Some(Button::Left),
        PointerButton::Secondary => Some(Button::Right),
        PointerButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

fn mods(m: Modifiers) -> (bool, bool, bool) {
    (m.ctrl || m.mac_cmd, m.shift, m.alt)
}

/// This frame's pointer input to the editor shown in `rect` (`k` editor pixels per point); `hovered`: the pointer is
/// on it, with nothing over it. A long press on the editor ([`gesture::is_long`], in the calm zone) is sent as a
/// right click (`Input::Down` with [`Button::Right`]), which opens the editor's own menu (Reset, …) on the widget
/// under the finger, as a secondary click does on the desktop.
fn forward(ui: &Ui, card: &mut LemonCard, rect: Rect, k: f32, hovered: bool, m: &Metrics) {
    let editor = card.editor;
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let (events, now, held) = ui.input(|i| (i.events.clone(), i.time, i.modifiers));
    let send = |e: Input, md: Modifiers| embed::input(editor, e, mods(md));
    for event in events {
        match event {
            Event::PointerMoved(p) => {
                if card.pressed.is_some() && embed::locked(editor) {
                    let d = (p - card.last) * k;
                    send(Input::Delta { dx: d.x, dy: d.y }, held);
                } else if card.pressed.is_some() || (hovered && rect.contains(p)) {
                    let (x, y) = px(p);
                    send(Input::Move { x, y }, held);
                    card.over = true;
                } else if std::mem::take(&mut card.over) {
                    send(Input::Leave, held);
                }
                card.last = p;
            }
            Event::PointerButton {
                pos,
                button: b,
                pressed: true,
                modifiers,
            } if card.pressed.is_none() && hovered && rect.contains(pos) => {
                let Some(btn) = button(b) else { continue };
                let dbl = (card.click)
                    .is_some_and(|(t, at)| now - t < DOUBLE_S && at.distance(pos) < DOUBLE_PT);
                card.click = (!dbl).then_some((now, pos));
                let (x, y) = px(pos);
                send(Input::Down { x, y, btn, dbl }, modifiers);
                (card.pressed, card.last, card.over) = (Some(btn), pos, true);
                card.long_menu = false;
                card.long_cancel = false;
            }
            Event::PointerButton {
                button: b,
                pressed: false,
                modifiers,
                ..
            } if card.pressed.is_some() && card.pressed == button(b) => {
                let btn = card.pressed.take().expect("held");
                // The long press's right click that opened no modal (a menu's answer replays it when one did; the
                // left up after a long press repeats the one sent with the right click, harmlessly).
                if card.long_menu {
                    send(Input::Up { btn: Button::Right }, modifiers);
                    card.long_menu = false;
                }
                send(Input::Up { btn }, modifiers);
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
            } if hovered && rect.contains(card.last) => {
                let notches = match unit {
                    MouseWheelUnit::Point => delta.y / WHEEL_POINTS,
                    MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
                };
                let (x, y) = px(card.last);
                send(Input::Wheel { x, y, notches }, modifiers);
            }
            Event::PointerGone if card.pressed.is_none() && std::mem::take(&mut card.over) => {
                send(Input::Leave, held);
            }
            _ => {}
        }
    }
    // A long press on the editor is a right click: once per press, held [`gesture::LONG_PRESS_S`] in the calm zone
    // about where it went down. Where it lands on a widget with a menu (a knob, a fader), the editor asks for the
    // menu ([`embed::modal`]); where it lands on nothing, the click is paired on release.
    if card.pressed == Some(Button::Left) && !card.long_menu && !card.long_cancel {
        let (origin, pos, start, now_s, second_finger) = ui.input(|i| {
            (
                i.pointer.press_origin(),
                i.pointer.latest_pos(),
                i.pointer.press_start_time(),
                i.time,
                i.multi_touch().is_some(),
            )
        });
        if let (Some(origin), Some(pos), Some(start)) = (origin, pos, start)
            && rect.contains(origin)
        {
            let held_s = now_s - start;
            if second_finger || gesture::left_calm(origin, Some(pos), m.u(1.0)) {
                // A second finger, or the calm zone left before its long press: never a menu (as [`gesture::press`]
                // cancels it).
                card.long_cancel = true;
            } else if gesture::is_long(held_s) {
                let (x, y) = px(origin);
                send(Input::Down { x, y, btn: Button::Right, dbl: false }, held);
                // The left press is released with it: where the right click opened a menu, its answer replays the
                // right click only, and a cancelled menu replays nothing — either way no up would reach the left press.
                send(Input::Up { btn: Button::Left }, held);
                card.long_menu = true;
            } else {
                // Wake when the long press comes due, as [`gesture::press`] does.
                ui.ctx().request_repaint_after_secs((gesture::LONG_PRESS_S - held_s) as f32);
            }
        }
    }
}

/// One level of an open menu: the menu, its layout, its top left in editor pixels, the row under the pointer, and its
/// painted image.
struct Level {
    menu: MenuView,
    layout: MenuLayout,
    at: Vec2,
    hover: Option<usize>,
    texture: Option<(TextureHandle, Option<usize>)>,
}

impl Level {
    fn new(menu: MenuView, at: Vec2, scale: f32) -> Option<Level> {
        let mut measure = Canvas::new(8, 8, scale).ok()?;
        let layout = kmenu::layout(&mut measure, &menu.items, menu.style);
        Some(Level {
            menu,
            layout,
            at,
            hover: None,
            texture: None,
        })
    }

    /// Its size in editor pixels.
    fn size(&self, scale: f32) -> Vec2 {
        vec2(
            (self.layout.w * scale).ceil(),
            (self.layout.h * scale).ceil(),
        )
    }

    /// The clickable row at editor pixel `p`.
    fn row_at(&self, p: Vec2, scale: f32) -> Option<usize> {
        let q = (p - self.at) / scale;
        let row = self.layout.item_at(&self.menu.items, q.x, q.y)?;
        self.menu.items[row].clickable().then_some(row)
    }
}

/// An open menu: the modal it shows, its levels (the menu, then each submenu open), and when it opened.
struct MenuUi {
    shown: (i32, i32, MenuView),
    levels: Vec<Level>,
    opened: f64,
    moved: bool,
}

/// Shows the editor's menu `menu` at editor pixel `at` over the editor in `rect`, and answers it: a row picked, or
/// cancelled by a press outside or Escape.
#[allow(clippy::too_many_arguments)]
fn show_menu(
    ctx: &egui::Context,
    id: Id,
    card: &mut LemonCard,
    rect: Rect,
    k: f32,
    at: (i32, i32),
    menu: MenuView,
    scale: f32,
) {
    let editor = card.editor;
    let now = ctx.input(|i| i.time);
    let key = (at.0, at.1, menu.clone());
    if card.menu.as_ref().is_none_or(|m| m.shown != key) {
        let Some(root) = Level::new(menu, vec2(at.0 as f32, at.1 as f32), scale) else {
            return embed::answer(editor, None);
        };
        card.menu = Some(MenuUi {
            shown: key,
            levels: vec![root],
            opened: now,
            moved: false,
        });
    }
    let m = card.menu.as_mut().expect("just set");
    let screen = ctx.content_rect();
    // A level's rect in points, kept on the screen.
    let place = |l: &Level| {
        let size = l.size(scale) / k;
        let min = rect.min + l.at / k;
        let min = min.min(screen.max - size).max(screen.min);
        Rect::from_min_size(min, size)
    };
    // A finger's lift takes the pointer away in its own frame: where it was is where it lifted.
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let editor_px = |p: Pos2, l: &Level| l.at + (p - place(l).min) * k;
    if let Some(p) = pointer {
        m.moved |= ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        // The deepest level under the pointer takes the hover; a row with a submenu opens it.
        if let Some(depth) = (0..m.levels.len())
            .rev()
            .find(|&d| place(&m.levels[d]).contains(p))
        {
            let hover = m.levels[depth].row_at(editor_px(p, &m.levels[depth]), scale);
            m.levels[depth].hover = hover;
            let sub = hover.and_then(|r| m.levels[depth].menu.sub(r).cloned().map(|s| (r, s)));
            let open = m.levels.get(depth + 1).map(|l| l.menu.clone());
            match sub {
                Some((row, s)) if open.as_ref() != Some(&s) => {
                    m.levels.truncate(depth + 1);
                    let r = m.levels[depth].layout.rows[row];
                    let at =
                        m.levels[depth].at + vec2((r.x + r.w) * scale - 2.0, r.y * scale - 2.0);
                    m.levels.extend(Level::new(s, at, scale));
                }
                Some(_) => {}
                None => m.levels.truncate(depth + 1),
            }
        }
    }
    // Picks and cancels.
    let (pressed, released, escape) = ctx.input_mut(|i| {
        let escape = i.consume_key(Modifiers::NONE, Key::Escape);
        (i.pointer.any_pressed(), i.pointer.any_released(), escape)
    });
    let inside = pointer.is_some_and(|p| m.levels.iter().any(|l| place(l).contains(p)));
    if escape || (pressed && !inside) {
        card.menu = None;
        return embed::answer(editor, None);
    }
    // The release of the press that opened it does not pick.
    if released && inside && (m.moved || now - m.opened > DOUBLE_S) {
        let p = pointer.expect("inside");
        if let Some(depth) = (0..m.levels.len())
            .rev()
            .find(|&d| place(&m.levels[d]).contains(p))
        {
            let l = &m.levels[depth];
            if let Some(row) = l
                .row_at(editor_px(p, l), scale)
                .filter(|&r| l.menu.sub(r).is_none())
            {
                let mut path: Vec<usize> =
                    (m.levels[..depth].iter()).filter_map(|l| l.hover).collect();
                path.push(row);
                card.menu = None;
                return embed::answer(editor, Some(Answer::Pick(path)));
            }
        }
    }
    let palette = os_plugkit::palette();
    for (d, l) in m.levels.iter_mut().enumerate() {
        let painted = l.texture.as_ref().map(|(_, h)| *h);
        if painted != Some(l.hover) {
            let size = l.size(scale);
            let Ok(mut canvas) = Canvas::new(size.x as u32, size.y as u32, scale) else {
                continue;
            };
            canvas.begin_frame(None);
            let (items, layout, style) = (&l.menu.items, &l.layout, l.menu.style);
            kmenu::paint(
                &mut canvas,
                &palette,
                items,
                layout,
                style,
                0.0,
                0.0,
                l.hover,
                l.menu.current,
            );
            let mut px = vec![0u32; (size.x * size.y) as usize];
            canvas.copy_to_xrgb(&mut px);
            let img = image(&px, size.x as u32, size.y as u32);
            match &mut l.texture {
                Some((t, h)) => {
                    t.set(img, TextureOptions::LINEAR);
                    *h = l.hover;
                }
                None => {
                    l.texture = Some((
                        ctx.load_texture(format!("lemon menu {d}"), img, TextureOptions::LINEAR),
                        l.hover,
                    ))
                }
            }
        }
        let r = place(l);
        if let Some((t, _)) = &l.texture {
            Area::new(id.with(("menu", d)))
                .order(Order::Foreground)
                .fixed_pos(r.min)
                .show(ctx, |ui| {
                    ui.allocate_rect(r, Sense::click_and_drag());
                    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
                    ui.painter().image(t.id(), r, uv, Color32::WHITE);
                });
        }
    }
}

/// Shows the editor's "Type in value" box at editor pixel `at` over the editor in `rect`, starting with `text`: Enter
/// answers with what was typed, Escape or a press outside cancels.
#[allow(clippy::too_many_arguments)]
fn type_in(
    ctx: &egui::Context,
    id: Id,
    card: &mut LemonCard,
    rect: Rect,
    k: f32,
    at: (i32, i32),
    text: String,
    scale: f32,
) {
    let editor = card.editor;
    let fresh = card.typed.is_none();
    let typed = card.typed.get_or_insert(text);
    let width = TYPE_IN_W * scale / k;
    let min = rect.min + vec2(at.0 as f32, at.1 as f32) / k;
    let field = id.with("type in");
    let (mut done, mut cancel) = (false, false);
    let shown = Area::new(id.with("type in area"))
        .order(Order::Foreground)
        .fixed_pos(min)
        .show(ctx, |ui| {
            let mut r = ui.add(TextEdit::singleline(typed).id(field).desired_width(width));
            // The whole text starts selected: typing replaces it.
            if fresh {
                r.request_focus();
                let mut state = TextEditState::load(ui.ctx(), field).unwrap_or_default();
                let all = CCursorRange::two(CCursor::new(0), CCursor::new(typed.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), field);
            }
            // On Android the native text dialog is the keyboard (eframe's IME is off there), so the box alone
            // would show no keys.
            crate::android_text::edit(&mut r, typed, usize::MAX);
            done = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            cancel = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                || (r.lost_focus() && !done && !fresh);
        })
        .response
        .rect;
    let outside = ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !shown.contains(p))
    });
    if done {
        let text = card.typed.take().unwrap_or_default();
        embed::answer(editor, Some(Answer::Text(text)));
    } else if cancel || outside {
        card.typed = None;
        embed::answer(editor, None);
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{CentralPanel, Frame, RawInput, TouchDeviceId, TouchId, TouchPhase};
    use lemon_sampler::view;

    use super::*;
    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::plugins::builtin::LEMON_SAMPLER;
    use crate::song::{ChannelKind, Song};

    /// A finger as Android reports it (egui-winit): the touch, with the pointer's move and press; its moves; its lift,
    /// with the release and the pointer gone.
    fn finger(phase: TouchPhase, pos: Pos2) -> Vec<Event> {
        let touch = Event::Touch {
            device_id: TouchDeviceId(1),
            id: TouchId(1),
            phase,
            pos,
            force: None,
        };
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        match phase {
            TouchPhase::Start => vec![touch, Event::PointerMoved(pos), button(true)],
            TouchPhase::Move => vec![touch, Event::PointerMoved(pos)],
            _ => vec![touch, button(false), Event::PointerGone],
        }
    }

    /// A Lemon Sampler's card at the screen's top left, one editor pixel a point.
    struct Rig {
        ctx: egui::Context,
        card: LemonCard,
        /// Its channel's id and its module's instance id.
        ids: (i32, i32),
        time: f64,
    }

    impl Rig {
        fn new() -> Rig {
            let mut song = Song::default();
            song.add_channel(ChannelKind::Instrument);
            let channel = &mut song.channels[1];
            let id = channel.id;
            let module = plugins::add(&mut channel.rack, id, LEMON_SAMPLER.uid, 0).unwrap();
            plugins::sync(&song);
            let mut rig = Rig {
                ctx: egui::Context::default(),
                card: LemonCard::default(),
                ids: (id, module),
                time: 0.0,
            };
            // The editor opens, then paints.
            rig.frame(vec![]);
            rig.frame(vec![]);
            rig
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let (card, (channel, module)) = (&mut self.card, self.ids);
            let m = Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE);
            let _ = self.ctx.run(input, |ctx| {
                CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
                    let width = LEMON_SAMPLER.size.0 as f32;
                    let content = Rect::from_min_size(Pos2::ZERO, vec2(width, 500.0));
                    assert!(show(ui, content, channel, module, &LEMON_SAMPLER, card, &m));
                });
            });
        }

        fn tap(&mut self, at: Pos2) {
            self.frame(finger(TouchPhase::Start, at));
            self.frame(finger(TouchPhase::End, at));
        }

        /// The centre of row `row` of the menu shown (at scale 1, on the screen as it is).
        fn row(&self, row: usize) -> Pos2 {
            let level = &self.card.menu.as_ref().expect("a menu shows").levels[0];
            let r = level.layout.rows[row];
            Pos2::ZERO + level.at + vec2(r.cx(), r.cy())
        }
    }

    /// The centre of the sampler's view rect `r` (`view::rr`) in its editor's pixels.
    fn centre(r: os_ui::Rect) -> Pos2 {
        pos2(r.cx() - view::CROP_X as f32, r.cy() - view::CROP_Y as f32)
    }

    /// Issue #52: on a phone, a list's menu takes the row tapped, and the editor the next tap.
    #[test]
    fn a_finger_picks_a_list_row_and_taps_on() {
        let mut rig = Rig::new();
        let editor = rig.card.editor;
        // Time stretching's `Mode` list (`view::sample_tab`).
        let mode = centre(view::rr(456.0, 548.0, 102.0, 122.0));
        rig.tap(mode);
        assert!(matches!(embed::modal(editor), Some(Modal::Menu { .. })));
        for _ in 0..5 {
            rig.frame(vec![]);
        }
        let level = &rig.card.menu.as_ref().expect("the card shows it").levels[0];
        let row = (0..level.menu.items.len())
            .find(|&r| level.menu.items[r].clickable() && Some(r) != level.menu.current)
            .expect("another mode");
        let _ = plugins::idle();
        rig.tap(rig.row(row));
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "the tap picked the row");
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 1),
            "the pick set the mode"
        );
        rig.tap(mode);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "the next tap is the editor's"
        );
    }

    /// A long press on a knob is a right click: the editor's menu (Reset, …) opens at the knob, and a quick tap
    /// does not. The lift of the press that opened it does not pick; a press outside cancels.
    #[test]
    fn a_long_press_on_a_knob_opens_its_menu_but_a_tap_does_not() {
        let mut rig = Rig::new();
        let editor = rig.card.editor;
        // The sample page's `START OFFSET` knob, in editor pixels (the card shows the view cropped).
        let knob = pos2(45.0 - view::CROP_X as f32, 286.0 - view::CROP_Y as f32);
        // A tap: down, up 0.1 s later — no menu.
        rig.tap(knob);
        rig.frame(vec![]);
        assert!(embed::modal(editor).is_none(), "a tap opens no menu");
        // A long press: down, then 0.4 s of frames with the finger unmoved.
        rig.frame(finger(TouchPhase::Start, knob));
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "the long press opened the knob's menu"
        );
        assert!(rig.card.menu.is_some(), "the card shows it");
        // Lifting the finger that opened it does not pick the row under it.
        rig.frame(finger(TouchPhase::End, knob));
        rig.frame(vec![]);
        assert!(
            matches!(embed::modal(editor), Some(Modal::Menu { .. })),
            "the lift left the menu open"
        );
        // A press outside cancels it.
        rig.tap(pos2(700.0, 550.0));
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "the outside press cancelled");
        // A long press elsewhere but moving on opens nothing: the finger left the calm zone before 300 ms.
        rig.frame(finger(TouchPhase::Start, knob));
        rig.frame(finger(TouchPhase::Move, knob + vec2(30.0, 0.0)));
        for _ in 0..4 {
            rig.frame(vec![]);
        }
        rig.frame(finger(TouchPhase::End, knob + vec2(30.0, 0.0)));
        assert!(embed::modal(editor).is_none(), "a moved press is a drag, not a menu");
    }
}
