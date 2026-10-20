// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Honeydew (vendored addition): a Lemon plugin editor drawn into Honeydew's own window, on every platform. This is
//! the editor window API of `x11` and `mac` (the plugins' `gui`) over a frame and an input queue that the host keeps:
//! the editor paints into [`frame`], and the host hands it input with [`input`] and runs it with [`pump`].
//!
//! - **No thread of its own.** Everything runs on the thread that opened the editor: [`spawn`] runs the editor until it
//!   waits, and [`pump`] runs it again with the input since. With no input waiting, [`Window::next`] returns `None` and
//!   the editor parks itself ([`Window::parking`]) instead of ending. So it works in a browser too, which has no
//!   threads. A tick comes every 30 ms of pumping, and at once after a [`Waker::wake`] (any thread).
//! - **Menus and the type-in box don't block.** [`Window::popup`] and [`Window::type_in`] hand the host the request
//!   ([`modal`]) and return `None`. When the user answers ([`answer`]), the input that opened it comes again, and this
//!   time the call returns the answer; a chain (a menu's `Type in value...`, then the box) replays its earlier answers
//!   first.
//! - Positions are the editor's device pixels, also for popups (`to_root` keeps them).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};

use os_theme::Palette;
use os_ui::kit::menu::{MenuItem, MenuStyle};

pub use crate::input::{keysym, keysym_char, Button, Input};
use crate::time::{Duration, Instant};
use crate::MenuDef;

/// The host's id of an embedded editor: the window handle it gives the plugin.
#[cfg(target_os = "macos")]
pub type Parent = usize;
#[cfg(not(target_os = "macos"))]
pub type Parent = u32;

/// The editor's polling period (overlay expiry, meters), as on Linux.
const TICK: Duration = Duration::from_millis(30);

/// The editor's last painted frame: XRGB rows of `width` pixels.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    /// Counts the frames painted.
    pub version: u64,
}

/// A menu as the host shows it: [`MenuDef`] without its commands.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuView {
    pub items: Vec<MenuItem>,
    pub subs: Vec<(usize, MenuView)>,
    pub style: MenuStyle,
    pub current: Option<usize>,
}

impl MenuView {
    fn of<C>(def: &MenuDef<C>) -> MenuView {
        MenuView {
            items: def.items.clone(),
            subs: def.subs.iter().map(|(i, d)| (*i, MenuView::of(d))).collect(),
            style: def.style,
            current: def.current,
        }
    }

    /// The submenu of row `i`.
    pub fn sub(&self, i: usize) -> Option<&MenuView> {
        self.subs.iter().find(|(j, _)| *j == i).map(|(_, m)| m)
    }
}

/// What the editor asks the user, at (`x`, `y`) in its device pixels; `scale` is its display scale.
#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    Menu { x: i32, y: i32, menu: MenuView, scale: f32 },
    /// The "Type in value" box, starting with `text`.
    TypeIn { x: i32, y: i32, text: String, scale: f32 },
}

/// The user's answer to a [`Modal`].
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// The menu row picked: its row in the menu, then in each submenu on the way.
    Pick(Vec<usize>),
    Text(String),
}

/// A modal waiting for its answer: the input that opened it, and the answers given before it in the chain.
struct Pending {
    modal: Modal,
    trigger: Input,
    before: Vec<Answer>,
}

/// An input delivered again with the answers to give: `left` of its events are still to come.
struct Replay {
    answers: VecDeque<Answer>,
    used: Vec<Answer>,
    trigger: Input,
    left: usize,
}

#[derive(Default)]
struct Inner {
    events: VecDeque<Input>,
    /// Ctrl, Shift, Alt.
    mods: (bool, bool, bool),
    locked: bool,
    frame: Frame,
    size: (u32, u32),
    modal: Option<Pending>,
    replay: Option<Replay>,
}

/// One open editor, shared by its window, its waker and the host.
#[derive(Default)]
struct State {
    inner: Mutex<Inner>,
    woken: AtomicBool,
    closed: AtomicBool,
}

impl State {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

static EDITORS: LazyLock<Mutex<HashMap<Parent, Arc<State>>>> = LazyLock::new(Default::default);
static SCALES: LazyLock<Mutex<HashMap<Parent, f32>>> = LazyLock::new(Default::default);
static REPAINT: OnceLock<fn()> = OnceLock::new();

thread_local! {
    /// The parked editors of this thread, by id.
    static PARKED: RefCell<HashMap<Parent, (Arc<State>, Box<dyn FnOnce()>)>> = RefCell::new(HashMap::new());
}

fn get(id: Parent) -> Option<Arc<State>> {
    EDITORS.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
}

fn repaint() {
    if let Some(f) = REPAINT.get() {
        f();
    }
}

// -------------------------------------------------------------------------------------------------------------------
// The editor's side: the `gui` API

/// Wakes an editor from any thread: a parameter changed (look again).
#[derive(Clone)]
pub struct Waker(Arc<State>);

impl Waker {
    pub fn wake(&self) {
        self.0.woken.store(true, Ordering::Release);
        repaint();
    }
}

/// Where an editor keeps its open window's waker.
pub type Slot = Arc<Mutex<Option<Waker>>>;

/// A parameter changed: the open editor (if any) looks again.
pub fn notify(slot: &Slot) {
    if let Ok(s) = slot.lock() {
        if let Some(w) = s.as_ref() {
            w.wake();
        }
    }
}

/// The open editor. Dropping it closes the editor.
pub struct EditorThread {
    id: Parent,
    state: Arc<State>,
    slot: Slot,
}

impl Drop for EditorThread {
    fn drop(&mut self) {
        if let Ok(mut s) = self.slot.lock() {
            *s = None;
        }
        self.state.closed.store(true, Ordering::Release);
        let mut editors = EDITORS.lock().unwrap_or_else(|e| e.into_inner());
        if editors.get(&self.id).is_some_and(|s| Arc::ptr_eq(s, &self.state)) {
            editors.remove(&self.id);
        }
        drop(editors);
        // Parked on this thread, it goes now; on another, at that thread's next `pump`. At the thread's end the parked
        // editors may have gone already.
        let parked = PARKED.try_with(|p| p.borrow_mut().remove(&self.id)).ok().flatten();
        drop(parked);
    }
}

/// The display scale the host asked for editor `parent` ([`set_scale`]); 1 if none.
pub fn backing_scale(parent: Parent) -> f32 {
    SCALES.lock().unwrap_or_else(|e| e.into_inner()).get(&parent).copied().unwrap_or(1.0)
}

/// Opens editor `parent`, `size` device pixels, and runs `run` with its window until it waits for input.
pub fn spawn(parent: Parent, size: (u32, u32), slot: &Slot, run: impl FnOnce(Window) + Send + 'static) -> Option<EditorThread> {
    let state = Arc::new(State::default());
    {
        let mut i = state.lock();
        i.size = size;
        i.events.push_back(Input::Expose);
    }
    EDITORS.lock().unwrap_or_else(|e| e.into_inner()).insert(parent, state.clone());
    if let Ok(mut s) = slot.lock() {
        *s = Some(Waker(state.clone()));
    }
    run(Window { id: parent, state: state.clone(), tick: Instant::now(), last: None });
    Some(EditorThread { id: parent, state, slot: slot.clone() })
}

/// Parks an editor that waits for input ([`Window::parking`]).
pub struct Park {
    id: Parent,
    state: Arc<State>,
}

impl Park {
    /// Keeps `resume` (the editor's loop, from where it waits) for the next [`pump`].
    pub fn park(self, resume: impl FnOnce() + 'static) {
        let old = PARKED.try_with(|p| p.borrow_mut().insert(self.id, (self.state, Box::new(resume))));
        drop(old);
    }
}

/// The editor's window.
pub struct Window {
    id: Parent,
    state: Arc<State>,
    tick: Instant,
    /// The input being handled, which a modal asks again with its answer.
    last: Option<Input>,
}

impl Window {
    /// No input is waiting: a good moment to paint.
    pub fn idle(&self) -> bool {
        self.state.lock().events.is_empty()
    }

    /// Ctrl, Shift, Alt held at the last input.
    pub fn mods(&self) -> (bool, bool, bool) {
        self.state.lock().mods
    }

    /// The next input; `None` when none is waiting (the editor then parks, [`Self::parking`]) or the editor closes.
    pub fn next(&mut self) -> Option<Input> {
        if self.state.closed.load(Ordering::Acquire) {
            return None;
        }
        let mut i = self.state.lock();
        // A replay ends once its events are handled.
        if i.replay.as_ref().is_some_and(|r| r.left == 0) {
            i.replay = None;
        }
        if let Some(e) = i.events.pop_front() {
            if let Some(r) = &mut i.replay {
                r.left -= 1;
            }
            self.last = Some(e.clone());
            return Some(e);
        }
        drop(i);
        if self.state.woken.swap(false, Ordering::AcqRel) || self.tick.elapsed() >= TICK {
            self.tick = Instant::now();
            self.last = Some(Input::Tick);
            return Some(Input::Tick);
        }
        None
    }

    /// Where the editor parks when [`Self::next`] had nothing; `None` once it closes.
    pub fn parking(&self) -> Option<Park> {
        (!self.state.closed.load(Ordering::Acquire)).then(|| Park { id: self.id, state: self.state.clone() })
    }

    /// The painted frame (XRGB rows of `w`).
    pub fn blit(&self, px: &[u32], w: u32, h: u32) {
        let mut i = self.state.lock();
        let f = &mut i.frame;
        f.pixels.clear();
        f.pixels.extend_from_slice(px);
        (f.width, f.height) = (w, h);
        f.version += 1;
        drop(i);
        repaint();
    }

    pub fn resize(&self, w: u32, h: u32) {
        self.state.lock().size = (w, h);
    }

    /// Popups are placed in the editor's own pixels.
    pub fn to_root(&self, x: i32, y: i32) -> (i32, i32) {
        (x, y)
    }

    /// Runs a desktop dialog (a file dialog) at once.
    pub fn dialog<R: Send + 'static>(&self, f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        Some(f())
    }

    /// Files dropped on the editor: none come.
    pub fn accept_files(&self) {}

    /// No audio clipboard.
    pub fn copy_audio(&self, _wav: Vec<u8>) -> bool {
        false
    }

    pub fn paste_audio(&self) -> Option<Vec<u8>> {
        None
    }

    pub fn locked(&self) -> bool {
        self.state.lock().locked
    }

    /// Holds the cursor: the host sends movement as `Input::Delta` until [`Self::unlock`].
    pub fn lock(&mut self) {
        self.state.lock().locked = true;
    }

    pub fn unlock(&mut self) {
        self.state.lock().locked = false;
    }

    /// The menu's picked command: the answer when this input comes again with one, else `None` while the host shows
    /// the menu ([`modal`]).
    pub fn popup<C: Clone>(&mut self, x: i32, y: i32, def: &MenuDef<C>, _pal: &Palette, scale: f32) -> Option<C> {
        if crate::no_modal() {
            return None;
        }
        self.unlock();
        match self.answer() {
            Some(Answer::Pick(path)) => picked(def, &path),
            Some(Answer::Text(_)) => None,
            None => {
                self.ask(Modal::Menu { x, y, menu: MenuView::of(def), scale });
                None
            }
        }
    }

    /// The typed text, as [`Self::popup`].
    pub fn type_in(&mut self, x: i32, y: i32, text: &str, scale: f32) -> Option<String> {
        if crate::no_modal() {
            return None;
        }
        self.unlock();
        match self.answer() {
            Some(Answer::Text(t)) => Some(t),
            Some(Answer::Pick(_)) => None,
            None => {
                self.ask(Modal::TypeIn { x, y, text: text.to_owned(), scale });
                None
            }
        }
    }

    /// The next answer of the replay running, if any.
    fn answer(&mut self) -> Option<Answer> {
        let mut i = self.state.lock();
        let r = i.replay.as_mut()?;
        let a = r.answers.pop_front()?;
        r.used.push(a.clone());
        Some(a)
    }

    /// Hands the host `modal`, with the input that asked it (and the answers before it in a replay).
    fn ask(&mut self, modal: Modal) {
        let mut i = self.state.lock();
        let (trigger, before) = match &i.replay {
            Some(r) => (r.trigger.clone(), r.used.clone()),
            None => match self.last.clone() {
                Some(e) => (e, Vec::new()),
                None => return,
            },
        };
        i.modal = Some(Pending { modal, trigger, before });
        drop(i);
        repaint();
    }
}

/// The command of the row `path` names in `def`.
fn picked<C: Clone>(def: &MenuDef<C>, path: &[usize]) -> Option<C> {
    let (&last, subs) = path.split_last()?;
    let mut d = def;
    for &i in subs {
        d = d.sub(i)?;
    }
    d.items.get(last).filter(|it| it.clickable())?;
    d.cmds.get(last).cloned()
}

// -------------------------------------------------------------------------------------------------------------------
// The host's side

/// `f` is called (any thread) whenever an editor painted, asked something or was woken: the host draws a frame soon.
pub fn set_repaint(f: fn()) {
    let _ = REPAINT.set(f);
}

/// The display scale editor `id` paints at, where the plugin asks the window for it (macOS).
pub fn set_scale(id: Parent, scale: f32) {
    SCALES.lock().unwrap_or_else(|e| e.into_inner()).insert(id, scale);
}

/// Whether editor `id` is open.
pub fn is_open(id: Parent) -> bool {
    get(id).is_some()
}

/// Queues input for editor `id`, with the modifier keys (Ctrl, Shift, Alt) held.
pub fn input(id: Parent, e: Input, mods: (bool, bool, bool)) {
    if let Some(s) = get(id) {
        let mut i = s.lock();
        i.mods = mods;
        i.events.push_back(e);
    }
}

/// Runs editor `id` with the input queued since (on the thread that opened it). Parked editors that closed meanwhile
/// go.
pub fn pump(id: Parent) {
    let (run, gone) = PARKED.with_borrow_mut(|p| {
        let closed: Vec<Parent> = p.iter().filter(|(_, (s, _))| s.closed.load(Ordering::Acquire)).map(|(k, _)| *k).collect();
        let gone: Vec<_> = closed.into_iter().filter_map(|k| p.remove(&k)).collect();
        (p.remove(&id), gone)
    });
    drop(gone);
    if let Some((_, resume)) = run {
        resume();
    }
}

/// Whether editor `id` holds the cursor (a knob drag): send it movement as `Input::Delta`.
pub fn locked(id: Parent) -> bool {
    get(id).is_some_and(|s| s.lock().locked)
}

/// Editor `id`'s frame, when it painted one after version `since`: `f` gets it, and its version is returned.
pub fn frame(id: Parent, since: u64, f: impl FnOnce(&Frame)) -> Option<u64> {
    let s = get(id)?;
    let i = s.lock();
    (i.frame.version > since).then(|| {
        f(&i.frame);
        i.frame.version
    })
}

/// What editor `id` asks the user, if anything.
pub fn modal(id: Parent) -> Option<Modal> {
    get(id).and_then(|s| s.lock().modal.as_ref().map(|p| p.modal.clone()))
}

/// The user's answer to editor `id`'s [`modal`], `None` to cancel it: an answer delivers the input that asked again.
pub fn answer(id: Parent, answer: Option<Answer>) {
    let Some(s) = get(id) else { return };
    let mut i = s.lock();
    let Some(p) = i.modal.take() else { return };
    let Some(a) = answer else { return };
    let mut answers: VecDeque<Answer> = p.before.into();
    answers.push_back(a);
    // A press comes with its release.
    let mut events = vec![p.trigger.clone()];
    if let Input::Down { btn, .. } = p.trigger {
        events.push(Input::Up { btn });
    }
    for e in events.iter().rev() {
        i.events.push_front(e.clone());
    }
    i.replay = Some(Replay { answers, used: Vec::new(), trigger: p.trigger, left: events.len() });
}
