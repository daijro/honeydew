//! A built-in Lemon plugin's card ([`crate::plugins::builtin`]). Below the header the card shows the plugin's own
//! editor, drawn by the plugin's own code (`os_plugkit::embed`) into a texture at the card's width, its aspect kept,
//! the same on every platform.
//!
//! - The pointer over it is the editor's, in the editor's pixels: presses (a second one within 0.4 s and 12 points is a
//!   double click), moves, releases (also outside, while a press it took is held) and the wheel. While the editor holds
//!   the cursor (a knob drag) the moves go as movement and the cursor hides. Issue #68: a double click on a knob or
//!   fader resets it, as a double tap does Honeydew's own.
//! - The editor's menus show over the card in Lemon's own look (`os_ui::kit::menu`), with their submenus; its "Type in
//!   value" box is a text field there.
//! - Issue #52: where the editor has no file dialog (Android, the browser), the app's sample picker opens its sample
//!   ([`wants_file`]).
//! - Its edits reach the song as a plugin editor's do (`plugins::idle`): the song's params, undo, REC.

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Area, Color32, ColorImage, CursorIcon, Event, Id, Key, Modifiers, MouseWheelUnit, Order,
    PointerButton, Pos2, Rect, Response, Sense, TextEdit, TextureHandle, TextureOptions, Ui, Vec2,
    pos2, vec2,
};
use os_plugkit::embed::{self, Answer, Button, Input, MenuView, Modal};
use os_ui::Canvas;
use os_ui::kit::menu::{self as kmenu, MenuLayout};

use crate::plugins;
use crate::plugins::builtin::Builtin;
use crate::ui::{Finger, MOUSE, fingers, focus_lost};

/// Two presses closer than this (seconds and points) are a double click: a finger's second tap lands a little off.
const DOUBLE_S: f64 = 0.4;
const DOUBLE_PT: f32 = 12.0;
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
pub(crate) struct LemonCard {
    /// The editor shown ([`plugins::lemon_editor`]).
    editor: embed::Parent,
    texture: Option<TextureHandle>,
    /// The frame shown, its size in pixels and when it came.
    version: u64,
    painted: f64,
    size: (u32, u32),
    /// The button of the press the editor took: its moves and release are the editor's wherever they are.
    pressed: Option<Button>,
    /// That press is a finger's other than the pointer's ([`other_finger`]): its id and the pass it was last seen in.
    finger: Option<(u64, u64)>,
    /// The pointer's last position, and whether it was over the editor.
    last: Pos2,
    over: bool,
    /// The last press: when and where, for a double click.
    click: Option<(f64, Pos2)>,
    menu: Option<MenuUi>,
    /// The type-in box's text, while it is open.
    typed: Option<String>,
    /// The press that opened the type-in box (a text field opens it at a press) lasts: its release is not a click away.
    typing_press: bool,
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

/// Issue #68: the largest rect of `size`'s aspect inside `area`, centred in it: where full screen shows the editor.
pub(crate) fn fit(area: Rect, size: (u32, u32)) -> Rect {
    let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let k = (area.width() / w).min(area.height() / h);
    Rect::from_center_size(area.center(), vec2(w, h) * k)
}

/// The size whose aspect the card shows `plugin`'s editor at: its last frame's, as an editor may resize itself (Tesla,
/// from its grip or its saved size), else the plugin's own.
pub(crate) fn shown_size(card: Option<&LemonCard>, plugin: &Builtin) -> (u32, u32) {
    (card.map(|c| c.size))
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or_else(|| plugin.size())
}

/// Draws module `instance_id`'s editor (built-in plugin `plugin`, on the channel with id `channel_id`) in the card body
/// `content` (points). `false` while the plugin is not running: the caller shows the plugin card instead.
pub(crate) fn show(
    ui: &mut Ui,
    content: Rect,
    channel_id: i32,
    instance_id: i32,
    plugin: &Builtin,
    card: &mut LemonCard,
) -> bool {
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    // The editor at the card's width in device pixels, so that it is sharp ([`plugins::lemon_editor`] opens it again
    // only when that moves by more than a little), up to the largest texture (full screen on a big screen).
    let (w, h) = plugin.size();
    let most = ctx.input(|i| i.max_texture_side) as f32 / w.max(h).max(1) as f32;
    let scale = (content.width() * ppp / w as f32).max(0.5).min(most);
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
    let size = shown_size(Some(card), plugin);
    let rect = Rect::from_min_size(
        content.min,
        vec2(content.width(), body_height(size, content.width())),
    );
    // Global (an editor shows in one card at a time): tests and render harnesses find the card by its editor.
    let id = Id::new(("lemon editor", editor));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    // Editor pixels per point.
    let k = card.size.0 as f32 / rect.width();
    let modal = embed::modal(editor);
    if modal.is_none() && card.size.0 > 0 {
        other_finger(ui, card, &response, rect, k);
        forward(ui, card, rect, k, response.hovered());
    } else {
        // The press that asked is the modal's (its answer delivers it again, with its release): the next press is new.
        (card.pressed, card.finger) = (None, None);
    }
    embed::pump(editor);
    // Issue #72: only what changed goes into the texture (a whole frame is megabytes, a knob turn a small patch of
    // it); all of it into a new texture or at a new size.
    let (shown, whole) = (card.size, card.texture.is_none());
    let mut fresh = None;
    if let Some(v) = embed::frame(editor, card.version, |f| {
        let size = (f.width, f.height);
        let area = (whole || size != shown).then_some((0, 0, size.0, size.1));
        fresh = (area.or(f.changed)).map(|a| (size, a, image(&f.pixels, f.width, a)));
    }) {
        card.version = v;
        card.painted = ctx.input(|i| i.time);
    }
    if let Some((size, (x, y, ..), image)) = fresh {
        card.size = size;
        match &mut card.texture {
            Some(t) if size == shown => {
                t.set_partial([x as usize, y as usize], image, TextureOptions::LINEAR)
            }
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
    paint(ui, rect, card);
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

/// Paints the editor's last frame in `rect`, as it was: the card's while its editor shows full screen.
pub(super) fn paint(ui: &Ui, rect: Rect, card: &LemonCard) {
    if let Some(t) = &card.texture {
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        ui.painter().image(t.id(), rect, uv, Color32::WHITE);
    }
}

/// Cancels the editor's open menu or type-in box, as Escape over it would: whether one was open. The app's `close`
/// takes Escape (and Back) first.
pub(crate) fn dismiss(card: &LemonCard) -> bool {
    let open = matches!(
        embed::modal(card.editor),
        Some(Modal::Menu { .. } | Modal::TypeIn { .. })
    );
    if open {
        embed::answer(card.editor, None);
    }
    open
}

/// Whether the editor asked for a file to open since the last call: the app's sample picker answers it.
pub(crate) fn wants_file(card: &mut LemonCard) -> bool {
    std::mem::take(&mut card.wants_file)
}

/// Whether the pointer was over a Lemon editor this frame: the rack's wheel is then the editor's.
pub(super) fn over(card: &LemonCard) -> bool {
    card.over || card.pressed.is_some()
}

/// The pixels (x0, y0, x1, y1) of XRGB rows of `w` as an image.
fn image(px: &[u32], w: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> ColorImage {
    let size = [(x1 - x0) as usize, (y1 - y0) as usize];
    let mut pixels = Vec::with_capacity(size[0] * size[1]);
    for row in px
        .chunks_exact(w.max(1) as usize)
        .take(y1 as usize)
        .skip(y0 as usize)
    {
        let rgb = |&p: &u32| Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
        pixels.extend(row[x0 as usize..x1 as usize].iter().map(rgb));
    }
    ColorImage::new(size, pixels)
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

/// Issue #68: a finger other than the pointer's (egui's pointer follows the first finger down only) presses, moves and
/// lifts as the pointer does while no press holds the editor, so that a knob turns while another finger plays keys; the
/// pointer waits meanwhile ([`forward`]). A focus loss or a frame without the card lifts it.
fn other_finger(ui: &Ui, card: &mut LemonCard, response: &Response, rect: Rect, k: f32) {
    let (editor, pass) = (card.editor, ui.ctx().cumulative_pass_nr());
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let send = |e: Input| embed::input(editor, e, (false, false, false));
    let held = |card: &LemonCard, id| card.finger.is_some_and(|f| f.0 == id);
    if card
        .finger
        .is_some_and(|f| f.1 + 1 != pass || ui.input(focus_lost))
    {
        send(Input::Up { btn: Button::Left });
        (card.pressed, card.finger) = (None, None);
    }
    // The pointer's own finger, which [`forward`] takes.
    let pointer = ui.input(|i| i.pointer.primary_pressed()) && response.is_pointer_button_down_on();
    for f in fingers(ui, response.id) {
        match f {
            Finger::Down(id, at) if id != MOUSE && !pointer && card.pressed.is_none() => {
                let (x, y) = px(at);
                send(Input::Down {
                    x,
                    y,
                    btn: Button::Left,
                    dbl: false,
                });
                (card.pressed, card.finger, card.last) = (Some(Button::Left), Some((id, pass)), at);
            }
            Finger::Move(id, at) if held(card, id) => {
                let d = (at - card.last) * k;
                let (x, y) = px(at);
                let locked = embed::locked(editor);
                send(if locked {
                    Input::Delta { dx: d.x, dy: d.y }
                } else {
                    Input::Move { x, y }
                });
                card.last = at;
            }
            Finger::Up(id, _) if held(card, id) => {
                send(Input::Up { btn: Button::Left });
                (card.pressed, card.finger) = (None, None);
            }
            _ => {}
        }
    }
    if let Some(f) = card.finger.as_mut() {
        f.1 = pass;
    }
}

/// This frame's pointer input to the editor shown in `rect` (`k` editor pixels per point); `hovered`: the pointer is
/// on it, with nothing over it. Nothing while another finger holds the editor ([`other_finger`]).
fn forward(ui: &Ui, card: &mut LemonCard, rect: Rect, k: f32, hovered: bool) {
    let editor = card.editor;
    if card.finger.is_some() {
        return;
    }
    let px = |p: Pos2| ((p.x - rect.left()) * k, (p.y - rect.top()) * k);
    let (events, now, held) = ui.input(|i| (i.events.clone(), i.time, i.modifiers));
    let send = |e: Input, m: Modifiers| embed::input(editor, e, mods(m));
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
                // Issue #68: a double press that starts a drag (a knob or fader holds the cursor) resets it first, by
                // a middle click (every Lemon editor's reset; most ignore `dbl`), and the drag goes on from there.
                if dbl && btn == Button::Left {
                    embed::pump(editor);
                    if embed::locked(editor) {
                        let down = |btn| Input::Down {
                            x,
                            y,
                            btn,
                            dbl: false,
                        };
                        let middle = Button::Middle;
                        let reset = [
                            Input::Up { btn },
                            down(middle),
                            Input::Up { btn: middle },
                            down(btn),
                        ];
                        reset.into_iter().for_each(|e| send(e, modifiers));
                    }
                }
                (card.pressed, card.last, card.over) = (Some(btn), pos, true);
            }
            Event::PointerButton {
                button: b,
                pressed: false,
                modifiers,
                ..
            } if card.pressed.is_some() && card.pressed == button(b) => {
                let btn = card.pressed.take().expect("held");
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
            let img = image(&px, size.x as u32, (0, 0, size.x as u32, size.y as u32));
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
    let down = ctx.input(|i| i.pointer.any_down());
    let opening = card.typing_press;
    card.typing_press = (fresh || opening) && down;
    let typed = card.typed.get_or_insert(text);
    let width = TYPE_IN_W * scale / k;
    let min = rect.min + vec2(at.0 as f32, at.1 as f32) / k;
    let field = id.with("type in");
    let (mut done, mut cancel) = (false, false);
    let shown = Area::new(id.with("type in area"))
        .order(Order::Foreground)
        .fixed_pos(min)
        .show(ctx, |ui| {
            let r = ui.add(TextEdit::singleline(typed).id(field).desired_width(width));
            // The whole text starts selected: typing replaces it. The release of the press that opened it keeps it.
            if fresh || (opening && r.lost_focus()) {
                r.request_focus();
                let mut state = TextEditState::load(ui.ctx(), field).unwrap_or_default();
                let all = CCursorRange::two(CCursor::new(0), CCursor::new(typed.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), field);
            }
            done = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            cancel = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                || (r.lost_focus() && !done && !fresh && !opening);
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
    use crate::plugins::builtin::named;
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

    /// A Lemon Sampler's card (or `name`'s) at the screen's top left, one editor pixel a point.
    struct Rig {
        name: &'static str,
        /// The screen it fills, as full screen does ([`fit`]); `None`: one editor pixel a point at the top left.
        full: Option<Rect>,
        ctx: egui::Context,
        card: LemonCard,
        /// Its channel's id and its module's instance id.
        ids: (i32, i32),
        time: f64,
    }

    impl Rig {
        fn new(name: &'static str) -> Rig {
            let mut song = Song::default();
            song.add_channel(ChannelKind::Instrument);
            let channel = &mut song.channels[1];
            let id = channel.id;
            let module = plugins::add(&mut channel.rack, id, named(name).uid, 0).unwrap();
            plugins::sync(&song);
            let mut rig = Rig {
                name,
                full: None,
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
            let (card, (channel, module), name) = (&mut self.card, self.ids, self.name);
            let full = self.full;
            let _ = self.ctx.run(input, |ctx| {
                CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| {
                    let sampler = named(name);
                    let width = sampler.size().0 as f32;
                    let content = match full {
                        Some(screen) => fit(screen, shown_size(Some(card), sampler)),
                        None => Rect::from_min_size(Pos2::ZERO, vec2(width, 500.0)),
                    };
                    assert!(show(ui, content, channel, module, sampler, card));
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

    /// Issue #68: a double tap on a knob (Lemon Chorus's Delay tile) puts it back at its default, the second tap a few
    /// points off the first.
    #[test]
    fn a_double_tap_resets_a_knob() {
        let mut rig = Rig::new("Lemon Chorus");
        // The tile (`lemon_small_fx::chorus`: x 8…166, y 8…54).
        let at = pos2(87.0, 31.0);
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, at));
        let default = (plugins::idle().iter().find(|m| m.gesture == 0))
            .expect("a press")
            .value;
        for y in [20.0, 0.0] {
            rig.frame(finger(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, pos2(at.x, 0.0)));
        let last =
            |moves: &[plugins::PluginMove]| moves.iter().rfind(|m| m.gesture == 1).map(|m| m.value);
        let moves = plugins::idle();
        assert!(last(&moves).is_some_and(|v| v != default), "{moves:?}");
        rig.tap(at);
        rig.tap(at + vec2(6.0, 3.0));
        rig.frame(vec![]);
        assert_eq!(last(&plugins::idle()), Some(default));
    }

    /// Issue #68: full screen, the editor fills the screen as far as its aspect allows, centred, painted at that scale
    /// (sharp, not stretched); a drag on a knob there turns it and a double tap puts it back, where the knob shows.
    #[test]
    fn full_screen_fits_the_editor_and_maps_the_finger() {
        let mut rig = Rig::new("Lemon Chorus");
        let size = named("Lemon Chorus").size();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let rect = fit(screen, size);
        let aspect = size.0 as f32 / size.1 as f32;
        assert!((rect.aspect_ratio() - aspect).abs() < 1e-3, "{rect:?}");
        assert!(screen.expand(1e-3).contains_rect(rect), "{rect:?}");
        assert!(rect.width() == screen.width() || rect.height() == screen.height());
        assert!(
            (rect.center() - screen.center()).length() < 1e-3,
            "{rect:?}"
        );
        // Opened again at the new scale.
        rig.full = Some(screen);
        rig.frame(vec![]);
        rig.frame(vec![]);
        assert!(
            (rig.card.size.0 as f32 - rect.width()).abs() <= 1.0,
            "{:?}",
            rig.card.size
        );
        // The Delay tile (`lemon_small_fx::chorus`: x 8…166, y 8…54) where it shows.
        let at = rect.min + vec2(87.0, 31.0) * (rect.width() / size.0 as f32);
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, at));
        let default = (plugins::idle().iter().find(|m| m.gesture == 0))
            .expect("a press")
            .value;
        for dy in [10.0, 40.0] {
            rig.frame(finger(TouchPhase::Move, at - vec2(0.0, dy)));
        }
        rig.frame(finger(TouchPhase::End, at - vec2(0.0, 40.0)));
        let last =
            |moves: &[plugins::PluginMove]| moves.iter().rfind(|m| m.gesture == 1).map(|m| m.value);
        let moves = plugins::idle();
        assert!(last(&moves).is_some_and(|v| v != default), "{moves:?}");
        rig.tap(at);
        rig.tap(at + vec2(6.0, 3.0));
        rig.frame(vec![]);
        assert_eq!(last(&plugins::idle()), Some(default));
    }

    /// Issue #68: a second finger turns a knob while the first (egui's pointer) holds a key elsewhere, which then lifts
    /// and moves nothing in the editor.
    #[test]
    fn a_second_finger_turns_a_knob() {
        let mut rig = Rig::new("Lemon Chorus");
        let (at, key) = (pos2(87.0, 31.0), pos2(790.0, 590.0));
        let second = |phase, pos| {
            vec![Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(2),
                phase,
                pos,
                force: None,
            }]
        };
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, key));
        rig.frame(second(TouchPhase::Start, at));
        for y in [20.0, 0.0] {
            rig.frame(second(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, key));
        let moves = plugins::idle();
        let values = moves.iter().filter(|m| m.gesture == 1).count();
        assert!(
            values > 0 && !moves.iter().any(|m| m.gesture == 2),
            "{moves:?}"
        );
        rig.frame(second(TouchPhase::End, pos2(at.x, 0.0)));
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 2),
            "the lift ends the press"
        );
    }

    /// Rind's spike editor (Honeydew's own, `crates/rind`) takes the same touch gestures as the Lemon plugins': a double
    /// tap puts its knob back at its default, the second tap a few points off the first.
    #[test]
    fn a_double_tap_resets_rinds_knob() {
        let mut rig = Rig::new("Rind");
        let at = pos2(90.0, 100.0);
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, at));
        let default = (plugins::idle().iter().find(|m| m.gesture == 0))
            .expect("a press")
            .value;
        for y in [80.0, 60.0] {
            rig.frame(finger(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, pos2(at.x, 60.0)));
        let last =
            |moves: &[plugins::PluginMove]| moves.iter().rfind(|m| m.gesture == 1).map(|m| m.value);
        let moves = plugins::idle();
        assert!(last(&moves).is_some_and(|v| v != default), "{moves:?}");
        rig.tap(at);
        rig.tap(at + vec2(6.0, 3.0));
        rig.frame(vec![]);
        assert_eq!(last(&plugins::idle()), Some(default));
    }

    /// Rind's knob turns under a second finger while the first holds something elsewhere, which then lifts and moves
    /// nothing in the editor.
    #[test]
    fn a_second_finger_turns_rinds_knob() {
        let mut rig = Rig::new("Rind");
        let (at, key) = (pos2(90.0, 100.0), pos2(790.0, 590.0));
        let second = |phase, pos| {
            vec![Event::Touch {
                device_id: TouchDeviceId(1),
                id: TouchId(2),
                phase,
                pos,
                force: None,
            }]
        };
        let _ = plugins::idle();
        rig.frame(finger(TouchPhase::Start, key));
        rig.frame(second(TouchPhase::Start, at));
        for y in [80.0, 60.0] {
            rig.frame(second(TouchPhase::Move, pos2(at.x, y)));
        }
        rig.frame(finger(TouchPhase::End, key));
        let moves = plugins::idle();
        assert!(
            moves.iter().any(|m| m.gesture == 1) && !moves.iter().any(|m| m.gesture == 2),
            "{moves:?}"
        );
        rig.frame(second(TouchPhase::End, pos2(at.x, 60.0)));
        assert!(
            plugins::idle().iter().any(|m| m.gesture == 2),
            "the lift ends the press"
        );
    }

    /// Issue #52: on a phone, a list's menu takes the row tapped, and the editor the next tap.
    #[test]
    fn a_finger_picks_a_list_row_and_taps_on() {
        let mut rig = Rig::new("Lemon Sampler");
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

    /// A text field the editor opens a type-in for at a press (Lemon Formula Controller's
    /// formula) keeps it open past that press's release, for the finger's typing.
    #[test]
    fn a_text_fields_type_in_outlasts_the_tap_that_opened_it() {
        let mut rig = Rig::new("Lemon Formula Controller");
        let editor = rig.card.editor;
        // The formula field (`lemon_small_fx::formula_controller`: x 192…467, y 58…81), held a few frames.
        let at = pos2(330.0, 70.0);
        rig.frame(finger(TouchPhase::Start, at));
        rig.frame(vec![]);
        rig.frame(vec![]);
        rig.frame(finger(TouchPhase::End, at));
        for _ in 0..3 {
            rig.frame(vec![]);
        }
        assert!(matches!(embed::modal(editor), Some(Modal::TypeIn { .. })));
        // It has the keys: a formula, then Enter.
        let enter = Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        rig.frame(vec![Event::Text("t".into())]);
        rig.frame(vec![enter]);
        rig.frame(vec![]);
        assert_eq!(embed::modal(editor), None, "Enter answered it");
        // The editor took it: the next type-in starts from it.
        rig.frame(finger(TouchPhase::Start, at));
        rig.frame(vec![]);
        let typed = matches!(embed::modal(editor), Some(Modal::TypeIn { text, .. }) if text == "t");
        assert!(typed, "{:?}", embed::modal(editor));
    }
}
