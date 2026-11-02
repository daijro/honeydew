//! A built-in Lemon plugin's card ([`crate::plugins::builtin`]). Below the header the card shows the plugin's own
//! editor, drawn by the plugin's own code (`os_plugkit::embed`) into a texture at the card's width, its aspect kept,
//! the same on every platform.
//!
//! - The pointer over it is the editor's, in the editor's pixels: presses (a second one within 0.4 s and 4 px is a
//!   double click), moves, releases (also outside, while a press it took is held) and the wheel. While the editor holds
//!   the cursor (a knob drag) the moves go as movement and the cursor hides.
//! - The editor's menus show over the card in Lemon's own look (`os_ui::kit::menu`), with their submenus; its "Type in
//!   value" box is a text field there.
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

use crate::plugins;
use crate::plugins::builtin::Builtin;

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
    texture: Option<TextureHandle>,
    /// The frame shown, its size in pixels and when it came.
    version: u64,
    painted: f64,
    size: (u32, u32),
    /// The button of the press the editor took: its moves and release are the editor's wherever they are.
    pressed: Option<Button>,
    /// The pointer's last position, and whether it was over the editor.
    last: Pos2,
    over: bool,
    /// The last press: when and where, for a double click.
    click: Option<(f64, Pos2)>,
    menu: Option<MenuUi>,
    /// The type-in box's text, while it is open.
    typed: Option<String>,
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
    let rect = Rect::from_min_size(
        content.min,
        vec2(content.width(), body_height(plugin.size, content.width())),
    );
    let id = ui.id().with(("lemon editor", editor));
    let response = ui.interact(rect, id, Sense::click_and_drag());
    // Editor pixels per point.
    let k = card.size.0 as f32 / rect.width();
    let modal = embed::modal(editor);
    if modal.is_none() && card.size.0 > 0 {
        forward(ui, card, rect, k, response.hovered());
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
        None => (card.menu, card.typed) = (None, None),
    }
    let now = ctx.input(|i| i.time);
    let busy =
        now - card.painted < BUSY_S || over(card) || card.menu.is_some() || card.typed.is_some();
    ctx.request_repaint_after(if busy { TICK } else { REST });
    true
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
/// on it, with nothing over it.
fn forward(ui: &Ui, card: &mut LemonCard, rect: Rect, k: f32, hovered: bool) {
    let editor = card.editor;
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
    let pointer = ctx.input(|i| i.pointer.latest_pos());
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
            let r = ui.add(TextEdit::singleline(typed).id(field).desired_width(width));
            // The whole text starts selected: typing replaces it.
            if fresh {
                r.request_focus();
                let mut state = TextEditState::load(ui.ctx(), field).unwrap_or_default();
                let all = CCursorRange::two(CCursor::new(0), CCursor::new(typed.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), field);
            }
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
