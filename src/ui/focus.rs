//! Keyboard focus.
//!
//! Each pass the drawn controls register here in draw order ([`register`], [`interact`], [`container`]): a tree whose
//! child order is the draw order. The focus actions work on the last complete pass's tree, as egui's own focus does.
//! The state lives in egui's memory, so a widget reaches it through its `Ui`; the app calls [`begin_frame`] before
//! anything draws and [`end_frame`] after everything has.
//!
//! - **Targets** are the controls, and the containers with nothing registered in them (the editor grids), that are not
//!   zero-size; a hidden control does not draw, so it does not register. A container with controls passes the focus to
//!   its first one (its last when moving back): the order is depth-first.
//! - `focusNext` / `focusPrev` (Tab / Shift+Tab) step through the targets in tree order; `focusLeft` … `focusDown`
//!   move to the target with the least gap + misalignment ([`score`]).
//! - **The focused control is asked first**: at the start of the frame a focused knob or slider takes
//!   `fineIncrement`, `increment`, `fineDecrement`, `decrement` ([`take_step`]) and a focused button `open`
//!   ([`interact`]), before anything else reads the keys; so Ctrl+= is a step there and a zoom elsewhere.
//! - Any focus action turns the keyboard look on; a touch turns it off. In keyboard mode the cursor is drawn over
//!   everything ([`end_frame`]).
//!
//! - The root wraps at the end of its list; every container hands the step on to its parent.
//! - The arrows look at every target in the tree, not only the siblings; a tie keeps the earlier one in tree order;
//!   with no candidate the focus stays.
//! - An auto-repeat (a press of a key already held) is spent without acting: no move, step, press or blink
//!   ([`crate::keymap::Keymap::consume_press`]).
//! - With nothing focused, Tab and the arrows go to the first target and Shift+Tab to the last.
//! - A modal (a backdrop, or [`crate::ui::modal_shown`]) keeps the focus inside it: the targets are the ones drawn
//!   after it (above its backdrop's layer, when it has one). A focus left under it moves to its first target, and
//!   comes back when the focused one goes.
//! - A focused control that is not drawn in a pass loses the focus to the root (or to where it was before a modal);
//!   one just asked for ([`focus`]) gets a pass to draw first, as a panel shown and focused at once does.
//! - A touch (any pointer press) also moves the focus to the topmost target under it, or to the root: the keys then
//!   act on what was last touched, with the look off.
//! - While an egui text field has the keyboard, the focus keys are the field's (`focusLeft`/`Right` move its cursor).
//!   egui's own Tab / arrow focus is let go every frame, so Space and Enter never click a widget it picked.
//! - `open` (Enter) presses a focused button, toggle, spinner or row as a tap does. Disabled controls are no targets.
//! - `blinkCursor` turns the keyboard look on, so the flash shows.
//! - A target with nothing of it in view shows the cursor on the nearest edge of the area it would show in (its clip
//!   on the screen), squeezed flat there: an edge indicator pointing the way to it.

use std::f64::consts::TAU;

use eframe::egui::{
    Context, Event, Id, LayerId, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind,
    TouchPhase, Ui, response::Flags, text_edit::TextEditState,
};

use crate::keymap;
use crate::theme::Theme;

/// The containers the app registers, by name (`Id::new(RACK)`), for the panel actions (`showRack` …). The playlist and
/// the bottom editor under it are one each: the one holding the focus takes the editing keys (`ui/edit_keys.rs`).
pub const EDITOR: &str = "focus editor";
pub const PLAYLIST: &str = "focus playlist";
pub const MIXER: &str = "focus mixer";
pub const RACK: &str = "focus rack";
/// The keyboard panel, which `showTransport` shows, focuses and hides.
pub const TRANSPORT: &str = "focus transport";
/// The SONGS page's file list, where `showMainBrowser` puts the focus.
pub const SONGS: &str = "focus songs list";

/// The cursor's step toward its target per drawn frame.
const EASE: f32 = 0.33;
/// The pulse's period, in seconds.
const PULSE_S: f64 = 2.5;
/// How far the pulse grows the outer frame each side at its peak, in device pixels.
const GROW_PX: f32 = 3.0;
/// The outer frame's line width, in device pixels.
const OUTER_PX: f32 = 9.0;
/// How far inside the outer frame the inner one starts, and its line width, in device pixels.
const INNER_PX: f32 = 3.0;
/// The blink's decay per frame, and the v it stops below.
const BLINK_DECAY: f32 = 0.8;
const BLINK_END: f32 = 0.001;

/// What a registered control does with the focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A knob or slider: the increment actions step it ([`take_step`]).
    Value,
    /// A button, toggle, spinner, tab or row: `open` presses it ([`activated`]).
    Button,
    /// Holds controls ([`container`]); with none it is a target itself, as an editor grid is.
    Container,
}

/// A focus action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Next,
    Prev,
    Left,
    Right,
    Up,
    Down,
}

/// An `increment` / `decrement` press for the focused knob or slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub up: bool,
    /// `fineIncrement` / `fineDecrement`.
    pub fine: bool,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    id: Id,
    rect: Rect,
    /// The clip it was drawn with: what of `rect` shows.
    clip: Rect,
    kind: Kind,
    order: Order,
    parent: Option<usize>,
    /// A control registered inside it.
    filled: bool,
}

/// One pass's controls in draw order, and the topmost modal: its first node and the lowest layer order it covers.
#[derive(Clone, Debug, Default)]
struct Tree {
    nodes: Vec<Node>,
    modal: Option<(usize, Order)>,
}

impl Tree {
    fn index(&self, id: Id) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// Node `i` was drawn before the modal, or under its backdrop.
    fn under_modal(&self, i: usize) -> bool {
        (self.modal).is_some_and(|(first, order)| i < first || self.nodes[i].order < order)
    }

    /// Node `i` takes the focus: a control or an empty container, not zero-size, not under a modal.
    fn target(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        !n.filled && n.rect.width() > 0.0 && n.rect.height() > 0.0 && !self.under_modal(i)
    }

    fn targets(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        (0..self.nodes.len()).filter(|&i| self.target(i))
    }

    /// Node `i` is `ancestor` or inside it.
    fn within(&self, i: usize, ancestor: Id) -> bool {
        let mut at = Some(i);
        while let Some(i) = at {
            if self.nodes[i].id == ancestor {
                return true;
            }
            at = self.nodes[i].parent;
        }
        false
    }

    /// Where `dir` takes the focus from `from`.
    fn step(&self, from: Option<Id>, dir: Dir) -> Option<Id> {
        let at = from.and_then(|id| self.index(id));
        let to = match (dir, at) {
            (Dir::Prev, None) => self.targets().next_back(),
            (_, None) => self.targets().next(),
            (Dir::Next, Some(f)) => {
                (self.targets().find(|&i| i > f)).or_else(|| self.targets().next())
            }
            (Dir::Prev, Some(f)) => {
                (self.targets().rev().find(|&i| i < f)).or_else(|| self.targets().next_back())
            }
            (_, Some(f)) => {
                let from = self.nodes[f].rect;
                let mut best: Option<(f32, usize)> = None;
                for i in self.targets().filter(|&i| i != f) {
                    let score = score(from, self.nodes[i].rect, dir);
                    if let Some(s) = score
                        && best.is_none_or(|(b, _)| s < b)
                    {
                        best = Some((s, i));
                    }
                }
                Some(best.map_or(f, |(_, i)| i))
            }
        };
        to.map(|i| self.nodes[i].id)
    }

    /// The topmost target whose shown part holds `pos`: the last drawn.
    fn at(&self, pos: Pos2) -> Option<Id> {
        (self.targets().rev())
            .find(|&i| {
                self.nodes[i]
                    .rect
                    .intersect(self.nodes[i].clip)
                    .contains(pos)
            })
            .map(|i| self.nodes[i].id)
    }
}

/// The directional score of `to` seen from `from`: the gap past `from`'s edge in `dir`, plus the misalignment across
/// it, min(|start offset|, |end offset|) when `to` lies wholly to one side, else 0. `None` when `to` is not wholly past
/// the edge (gap < 0), and for Next / Prev.
fn score(from: Rect, to: Rect, dir: Dir) -> Option<f32> {
    let (gap, a, b) = match dir {
        Dir::Left => (from.left() - to.right(), from.y_range(), to.y_range()),
        Dir::Right => (to.left() - from.right(), from.y_range(), to.y_range()),
        Dir::Up => (from.top() - to.bottom(), from.x_range(), to.x_range()),
        Dir::Down => (to.top() - from.bottom(), from.x_range(), to.x_range()),
        Dir::Next | Dir::Prev => return None,
    };
    let aside = b.max <= a.min || b.min >= a.max;
    let misalignment = match aside {
        true => (b.min - a.min).abs().min((b.max - a.max).abs()),
        false => 0.0,
    };
    (gap >= 0.0).then_some(gap + misalignment)
}

/// The pulse p = 0.5 + 0.5·sin(2π·t / 2.5 s) at egui time `t`.
fn pulse(t: f64) -> f32 {
    (0.5 + 0.5 * (TAU * t / PULSE_S).sin()) as f32
}

/// The cursor rect one drawn frame on: 33 % of the way from `from` to `to`.
fn ease(from: Rect, to: Rect) -> Rect {
    Rect::from_min_max(
        from.min + (to.min - from.min) * EASE,
        from.max + (to.max - from.max) * EASE,
    )
}

/// The cursor's rect for a target at `rect` in the visible area `view`: itself while any of it shows, else clamped
/// onto `view`'s nearest edge (the edge indicator).
fn on_view(rect: Rect, view: Rect) -> Rect {
    if rect.intersects(view) || !view.is_positive() {
        rect
    } else {
        Rect::from_min_max(view.clamp(rect.min), view.clamp(rect.max))
    }
}

/// The outer and inner frames' rects around cursor `rect` at pulse `p`, `ppp` device pixels per point: the outer is
/// `rect` grown 3p px each side, the inner 3 px inside it. Each line is drawn inside its rect.
fn frames(rect: Rect, p: f32, ppp: f32) -> (Rect, Rect) {
    let outer = rect.expand(GROW_PX * p / ppp);
    (outer, outer.shrink(INNER_PX / ppp))
}

/// The blink's fill at v: (1 − v)·`rect`, centred, and its alpha √v.
fn flash(rect: Rect, v: f32) -> (Rect, f32) {
    (
        Rect::from_center_size(rect.center(), rect.size() * (1.0 - v)),
        v.sqrt(),
    )
}

#[derive(Clone, Default)]
struct State {
    pass: u64,
    /// This pass's tree, being drawn.
    tree: Tree,
    /// The containers open in `tree`, innermost last.
    open: Vec<usize>,
    /// The last complete pass's tree.
    last: Tree,
    focused: Option<Id>,
    /// Where the focus was before each modal took it, innermost last: each closing goes back one.
    back: Vec<Id>,
    keyboard: bool,
    /// The cursor as drawn last frame; `None` while hidden.
    cursor: Option<Rect>,
    /// The blink's v, 0 when not blinking.
    blink: f32,
    /// This frame's increment for the focused knob or slider.
    step: Option<(Id, Step)>,
    /// This frame's `open` for the focused button.
    press: Option<Id>,
    /// The pass [`focus`] was last asked in: a panel shown and focused at once draws from the next pass on.
    asked: Option<u64>,
    /// The target that takes `focusUp` / `focusDown` this frame while it is focused ([`hold_vertical`]).
    vertical: Option<Id>,
}

impl State {
    fn add(&mut self, mut node: Node) -> usize {
        node.parent = self.open.last().copied();
        if let Some(p) = node.parent {
            self.tree.nodes[p].filled = true;
        }
        self.tree.nodes.push(node);
        self.tree.nodes.len() - 1
    }

    /// The focus follows what drew: a container passes it to its first target, a modal takes it, and one whose
    /// control is gone goes back to where it was before the modal, else to the topmost modal's first target (an inner
    /// modal closing over an outer one, or a row gone inside a modal, whose opener waits under it), else to the root; one asked for in this pass waits a pass for its control to
    /// draw.
    fn settle(&mut self) {
        let (Some(id), tree) = (self.focused, &self.tree) else {
            return;
        };
        self.focused = match tree.index(id) {
            None if self.asked == Some(self.pass) => Some(id),
            None => {
                // The latest way back that is a target now. One under a modal still open stays for that modal's
                // closing; one gone is dropped.
                let mut to = None;
                while let Some(&b) = self.back.last() {
                    match tree.index(b) {
                        Some(i) if tree.target(i) => {
                            to = self.back.pop();
                            break;
                        }
                        Some(i) if tree.under_modal(i) => break,
                        _ => {
                            self.back.pop();
                        }
                    }
                }
                to.or_else(|| {
                    tree.modal
                        .and(tree.targets().next())
                        .map(|t| tree.nodes[t].id)
                })
            }
            Some(i) if tree.nodes[i].filled => {
                let inside = tree.targets().find(|&t| tree.within(t, id));
                Some(inside.map_or(id, |t| tree.nodes[t].id))
            }
            Some(i) if tree.under_modal(i) => match tree.targets().next() {
                Some(t) => {
                    self.back.push(id);
                    Some(tree.nodes[t].id)
                }
                None => Some(id),
            },
            Some(_) => Some(id),
        };
    }
}

/// The focus state, on a new pass with this pass's tree started afresh.
fn with<R>(ctx: &Context, f: impl FnOnce(&mut State) -> R) -> R {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<State>(Id::new("keyboard focus"));
        if s.pass != pass {
            std::mem::swap(&mut s.last, &mut s.tree);
            s.tree.nodes.clear();
            s.tree.modal = None;
            s.open.clear();
            s.pass = pass;
        }
        f(s)
    })
}

fn node(ui: &Ui, id: Id, rect: Rect, kind: Kind) -> Node {
    Node {
        id,
        rect,
        clip: ui.clip_rect(),
        kind,
        order: ui.layer_id().order,
        parent: None,
        filled: false,
    }
}

/// Registers control `id`, drawn in `rect`, in this pass's tree.
pub fn register(ui: &Ui, id: Id, rect: Rect, kind: Kind) {
    with(ui.ctx(), |s| s.add(node(ui, id, rect, kind)));
}

/// Registers container `id` in `rect`; what `add` registers is inside it.
pub fn container<R>(ui: &mut Ui, id: Id, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    open(ui, id, rect);
    let inner = add(ui);
    close(ui.ctx(), id);
    inner
}

/// [`container`] for drawing code too long to sit in a closure: what registers from here until [`close`] of the same
/// `id` is inside container `id`.
pub fn open(ui: &Ui, id: Id, rect: Rect) {
    let node = node(ui, id, rect, Kind::Container);
    with(ui.ctx(), |s| {
        let i = s.add(node);
        s.open.push(i);
    });
}

/// Ends container `id` that [`open`] began, and any still open inside it.
pub fn close(ctx: &Context, id: Id) {
    with(ctx, |s| {
        if let Some(at) = s.open.iter().position(|&o| s.tree.nodes[o].id == id) {
            s.open.truncate(at);
        }
    });
}

/// `ui.interact` for a button-like control: one that senses clicks (and is enabled) is a [`Kind::Button`] target, and
/// `open` on it is a click (egui's keyboard click: `clicked()` is true, with no pointer position). A mouse over it shows
/// the clickable cursor.
pub fn interact(ui: &Ui, rect: Rect, id: Id, sense: Sense) -> Response {
    let mut response = ui.interact(rect, id, sense);
    if sense.senses_click() && ui.is_enabled() {
        if response.hovered() {
            ui.ctx()
                .set_cursor_icon(eframe::egui::CursorIcon::PointingHand);
        }
        register(ui, id, rect, Kind::Button);
        if activated(ui.ctx(), id) {
            response.flags.set(Flags::FAKE_PRIMARY_CLICKED, true);
        }
    }
    response
}

/// `open` pressed this frame on control `id`, the focused button; taken.
pub fn activated(ctx: &Context, id: Id) -> bool {
    with(ctx, |s| s.press == Some(id) && s.press.take().is_some())
}

/// The increment pressed this frame for control `id`, the focused knob or slider; taken.
pub fn take_step(ctx: &Context, id: Id) -> Option<Step> {
    with(ctx, |s| match s.step {
        Some((f, step)) if f == id => s.step.take().map(|_| step),
        _ => None,
    })
}

/// Target `id` takes `focusUp` / `focusDown` in the next frame while it is focused, as itself said in this one (an
/// editor grid nudging its selection).
pub fn hold_vertical(ctx: &Context, id: Id) {
    with(ctx, |s| s.vertical = Some(id));
}

/// Focuses control or container `id` (a container passes it to its first target at the frame's end); keyboard look
/// on.
pub fn focus(ctx: &Context, id: Id) {
    move_to(ctx, id);
    with(ctx, |s| s.keyboard = true);
}

/// [`focus`], keeping the keyboard look as it is: a tap's move (the bottom editors' close X).
pub fn move_to(ctx: &Context, id: Id) {
    with(ctx, |s| {
        s.focused = Some(id);
        s.asked = Some(s.pass);
    });
}

/// [`move_to`] into a modal that is opening: where the focus was is kept, and it goes back there when the modal closes
/// (as when the modal takes it, `State::settle`). Entering where the focus already is keeps nothing.
pub fn enter(ctx: &Context, id: Id) {
    with(ctx, |s| s.back.extend(s.focused.filter(|&f| f != id)));
    move_to(ctx, id);
}

/// The latest way back is dropped: a pick in a modal moved the focus on into what is no modal (the chord editor), so
/// nothing closing later sends it back to the modal's opener.
pub fn drop_way_back(ctx: &Context) {
    with(ctx, |s| s.back.pop());
}

/// The focus goes back to the app root.
pub fn clear(ctx: &Context) {
    with(ctx, |s| s.focused = None);
}

pub fn focused(ctx: &Context) -> Option<Id> {
    with(ctx, |s| s.focused)
}

/// How many ways back ([`enter`], a modal taking the focus) are kept.
#[cfg(test)]
pub(crate) fn ways_back(ctx: &Context) -> usize {
    with(ctx, |s| s.back.len())
}

/// The focus is on container `id` or inside it (as of the last complete pass).
pub fn within(ctx: &Context, id: Id) -> bool {
    with(ctx, |s| {
        let at = s.focused.and_then(|f| s.last.index(f));
        s.focused == Some(id) || at.is_some_and(|i| s.last.within(i, id))
    })
}

/// A focus action: moves the focus `dir` and turns the keyboard look on.
pub fn move_dir(ctx: &Context, dir: Dir) {
    with(ctx, |s| {
        s.focused = s.last.step(s.focused, dir);
        s.keyboard = true;
    });
}

/// The keyboard-navigation look is on: the cursor shows.
pub fn is_keyboard_mode(ctx: &Context) -> bool {
    with(ctx, |s| s.keyboard)
}

/// `blinkCursor`: flashes the cursor, and turns the keyboard look on.
pub fn blink(ctx: &Context) {
    with(ctx, |s| {
        s.blink = 1.0;
        s.keyboard = true;
    });
}

/// A modal shows from here on in this pass: only what it draws next, in layers of order `lowest` or above, takes the
/// focus. The latest call is the topmost modal.
pub(crate) fn modal(ctx: &Context, lowest: Order) {
    with(ctx, |s| s.tree.modal = Some((s.tree.nodes.len(), lowest)));
}

/// The focus actions at the frame's start, before anything draws or reads the keys: a touch, the focus keys, then
/// the focused control's own keys, which it is asked for first.
pub fn begin_frame(ctx: &Context) {
    // egui moves a focus of its own on Tab and the arrows, and clicks it on Space or Enter; a text field keeps it.
    // ponytail: let go every frame, so a screen reader cannot rest egui's focus on a widget that is not a text field;
    // let go only in frames with a Space, Enter, Tab or arrow press if AccessKit focus matters.
    let typing = match ctx.memory(|m| m.focused()) {
        Some(id) if TextEditState::load(ctx, id).is_some() => true,
        Some(id) => {
            ctx.memory_mut(|m| m.surrender_focus(id));
            false
        }
        None => false,
    };
    let touch = ctx.input(|i| {
        i.events.iter().find_map(|e| match *e {
            Event::PointerButton {
                pos, pressed: true, ..
            } => Some(pos),
            Event::Touch {
                phase: TouchPhase::Start,
                pos,
                ..
            } => Some(pos),
            _ => None,
        })
    });
    with(ctx, |s| {
        (s.step, s.press) = (None, None);
        if let Some(pos) = touch {
            s.keyboard = false;
            s.focused = s.last.at(pos);
            s.back.clear();
        }
    });
    if typing {
        return;
    }
    let keys = keymap::for_os(ctx.os());
    // The most specific first: a binding ignores extra Shift and Alt.
    const MOVES: [(&str, Dir); 6] = [
        ("focusPrev", Dir::Prev),
        ("focusNext", Dir::Next),
        ("focusLeft", Dir::Left),
        ("focusRight", Dir::Right),
        ("focusUp", Dir::Up),
        ("focusDown", Dir::Down),
    ];
    // A focused target holding the arrows keeps Up and Down.
    let held = with(ctx, |s| {
        s.vertical.take().is_some_and(|v| s.focused == Some(v))
    });
    let moves: Vec<Dir> = ctx.input_mut(|i| {
        (MOVES.iter())
            .filter(|(_, dir)| !(held && matches!(dir, Dir::Up | Dir::Down)))
            .filter(|(action, _)| keys.consume_press(i, action))
            .map(|&(_, dir)| dir)
            .collect()
    });
    for dir in moves {
        move_dir(ctx, dir);
    }
    if ctx.input_mut(|i| keys.consume_press(i, "blinkCursor")) {
        blink(ctx);
    }
    let kind = with(ctx, |s| {
        let at = s.focused.and_then(|f| s.last.index(f));
        // Not one under a modal (with no target of its own to take the focus).
        at.filter(|&i| !s.last.under_modal(i))
            .map(|i| s.last.nodes[i].kind)
    });
    match kind {
        Some(Kind::Value) => {
            const STEPS: [(&str, Step); 4] = [
                (
                    "fineIncrement",
                    Step {
                        up: true,
                        fine: true,
                    },
                ),
                (
                    "increment",
                    Step {
                        up: true,
                        fine: false,
                    },
                ),
                (
                    "fineDecrement",
                    Step {
                        up: false,
                        fine: true,
                    },
                ),
                (
                    "decrement",
                    Step {
                        up: false,
                        fine: false,
                    },
                ),
            ];
            let step = ctx.input_mut(|i| {
                (STEPS.iter())
                    .find(|(action, _)| keys.consume_press(i, action))
                    .map(|&(_, step)| step)
            });
            with(ctx, |s| s.step = s.focused.zip(step));
        }
        Some(Kind::Button) => {
            if ctx.input_mut(|i| keys.consume_press(i, "open")) {
                with(ctx, |s| s.press = s.focused);
            }
        }
        Some(Kind::Container) | None => {}
    }
}

/// The frame's end, after everything drew: the focus follows its control (`State::settle`), and in keyboard mode
/// the cursor is drawn over everything: the focused control's rect, eased 33 % a frame toward it (at once when it first
/// shows), an outer frame grown 3p px each side with a 9 px line in the theme colour that contrasts best with `text`,
/// an inner frame 3 px inside it with a 3 px `text` line, and the blink's flash.
pub fn end_frame(ctx: &Context, theme: &Theme) {
    let (ppp, time, screen) = (
        ctx.pixels_per_point(),
        ctx.input(|i| i.time),
        ctx.content_rect(),
    );
    let shown = with(ctx, |s| {
        (s.step, s.press) = (None, None);
        s.settle();
        let at = s
            .focused
            .and_then(|f| s.tree.index(f))
            .filter(|_| s.keyboard);
        let target = at
            .map(|i| s.tree.nodes[i])
            .map(|n| on_view(n.rect, n.clip.intersect(screen)));
        s.cursor = target.map(|r| s.cursor.map_or(r, |c| ease(c, r)));
        let v = s.blink;
        s.blink = if v * BLINK_DECAY < BLINK_END {
            0.0
        } else {
            v * BLINK_DECAY
        };
        s.cursor.map(|c| (c, v))
    });
    let Some((rect, v)) = shown else {
        return;
    };
    // The pulse moves every frame.
    ctx.request_repaint();
    let c = &theme.colors;
    let text = c.text.to_color32();
    let outer_color = c.best_contrasting(c.text, &c.contrast_candidates());
    let (outer, inner) = frames(rect, pulse(time), ppp);
    let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("focus cursor")));
    let stroke = |px: f32, color| Stroke::new(px / ppp, color);
    painter.rect_stroke(
        outer,
        0.0,
        stroke(OUTER_PX, outer_color.to_color32()),
        StrokeKind::Inside,
    );
    painter.rect_stroke(inner, 0.0, stroke(INNER_PX, text), StrokeKind::Inside);
    if v > 0.0 {
        let (fill, alpha) = flash(rect, v);
        painter.rect_filled(fill, 0.0, text.gamma_multiply(alpha));
    }
}

#[cfg(test)]
mod tests;
