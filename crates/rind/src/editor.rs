// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Rind's editor: the control logic (platform independent: drags with the cursor held in place, taps, the wheel, reset
//! gestures) and, with the `embed` feature, the window glue that runs it in Honeydew's own window
//! (`os_plugkit::embed`).
//!
//! The glue follows the template of Lemon Studio's editors (`lemon_small_fx::editor`): an input loop over the window's
//! inputs that ends when none is waiting, where the editor parks (`os_plugkit::park!`) and Honeydew runs it on when it
//! brings more. Edits go through `os_plugkit::vals::Vals`, so they show and sound at once ([`crate::params::RindParams::live`])
//! and reach the host as gestures.
//!
//! Reset gestures: Honeydew's card turns a double tap on a control that holds the cursor (a knob or fader) into a middle
//! click before the drag goes on, so the middle button and Alt-click reset to the default here, and a press never
//! looks at `dbl`.
// Without `embed` there is no window to run the logic: only the tests do.
#![cfg_attr(not(feature = "embed"), allow(dead_code))]

use std::any::Any;
use std::sync::Arc;
#[cfg(feature = "embed")]
use std::sync::atomic::{AtomicU32, Ordering};

use nih_plug::prelude::*;
use os_plugkit::vals::Vals;
use os_ui::Canvas;

use crate::ids::{Kind as Range, SPECS};
use crate::params::RindParams;
use crate::view::{self, Hit, Kind, Ui, Values};

/// The modifier keys that matter here, as held at an input: Ctrl (fine steps) and Alt (reset).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Left,
    Right,
    Middle,
}

/// What the window must do after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Repaint,
    /// Hold the cursor: movement comes as deltas until the press ends.
    Lock,
}

/// Which way a drag runs: up raises a knob or bar, right raises a slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Up,
    Right,
}

/// A drag: `v` is the unrounded normalised value it has reached, over `px` for the whole range.
#[derive(Clone, Copy, Debug)]
struct Drag {
    param: usize,
    v: f32,
    px: f32,
    axis: Axis,
}

/// The values of a [`Vals`], by table index.
struct Shown<'a>(&'a Vals);

impl Values for Shown<'_> {
    fn norm(&self, p: usize) -> f32 {
        self.0.norm(SPECS[p].id)
    }
    fn plain(&self, p: usize) -> f32 {
        self.0.plain(SPECS[p].id)
    }
    fn text(&self, p: usize) -> String {
        self.0.text(SPECS[p].id)
    }
}

/// The editor's logic.
pub struct Logic {
    pub vals: Vals,
    ui: Ui,
    hits: Vec<Hit>,
    drag: Option<Drag>,
    pressed: Option<usize>,
    last: Option<usize>,
}

/// A fine drag or wheel step (Ctrl) is this share of a normal one.
const FINE: f32 = 0.25;
/// A wheel notch moves a knob or slider this much of its range.
const WHEEL_STEP: f32 = 0.02;

impl Logic {
    pub fn new(vals: Vals) -> Self {
        let ui = Ui::default();
        Logic {
            vals,
            hits: view::layout(&ui),
            ui,
            drag: None,
            pressed: None,
            last: None,
        }
    }

    /// Paints the editor and keeps its hit regions.
    pub fn paint(&mut self, c: &mut Canvas) {
        let hits = view::paint(c, &Shown(&self.vals), &self.ui, self.pressed, self.last);
        self.hits = hits;
    }

    /// The regions of the screen as it is now (after a tab, sub tab or list changed).
    fn relayout(&mut self) {
        self.hits = view::layout(&self.ui);
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|h| h.r.contains(x, y)).copied()
    }

    fn reset(&mut self, p: usize) {
        let id = SPECS[p].id;
        let default = self.vals.default_norm(id);
        self.vals.edit(id, default);
    }

    fn begin_drag(&mut self, p: usize, px: f32, axis: Axis, at: Option<f32>) -> Vec<Act> {
        let id = SPECS[p].id;
        self.vals.begin(id);
        if let Some(v) = at {
            self.vals.set(id, v);
        }
        self.drag = Some(Drag {
            param: p,
            v: at.unwrap_or_else(|| self.vals.norm(id)),
            px,
            axis,
        });
        self.pressed = Some(p);
        vec![Act::Repaint, Act::Lock]
    }

    /// The next (`step` 1) or previous (-1) whole value of `p`, wrapping when `wrap`.
    fn step(&mut self, p: usize, step: i32, wrap: bool) {
        let Range::Int(lo, hi) = SPECS[p].kind else {
            return;
        };
        let now = self.vals.plain(SPECS[p].id).round() as i32;
        let mut to = now + step;
        if to > hi {
            to = if wrap { lo } else { hi };
        }
        if to < lo {
            to = if wrap { hi } else { lo };
        }
        self.vals.edit_plain(SPECS[p].id, to as f32);
    }

    /// A press at (`x`, `y`) in logical px.
    pub fn down(&mut self, x: f32, y: f32, btn: Btn, m: Mods) -> Vec<Act> {
        let Some(h) = self.hit(x, y) else {
            return Vec::new();
        };
        let p = h.param;
        match h.kind {
            Kind::Tab(i) => {
                if btn == Btn::Left {
                    self.ui.tab = i;
                    self.relayout();
                    return vec![Act::Repaint];
                }
                return Vec::new();
            }
            Kind::Sub(i) => {
                if btn == Btn::Left {
                    let tab = self.ui.tab;
                    self.ui.subs[tab] = i;
                    self.relayout();
                    return vec![Act::Repaint];
                }
                return Vec::new();
            }
            // A name in the open list: pick it and close the list (only a left press picks).
            Kind::Item(i) => {
                if btn == Btn::Left {
                    let (_, lo) = view::choices(p);
                    self.vals.edit_plain(SPECS[p].id, (lo + i as i32) as f32);
                    self.last = Some(p);
                }
                self.ui.popup = None;
                self.relayout();
                return vec![Act::Repaint];
            }
            // Anywhere else: the list closes and the press goes no further.
            Kind::Dismiss => {
                self.ui.popup = None;
                self.relayout();
                return vec![Act::Repaint];
            }
            _ => {}
        }
        self.last = Some(p);
        match (btn, m.alt) {
            (Btn::Middle, _) | (Btn::Left, true) => self.reset(p),
            (Btn::Right, _) => {
                if h.kind == Kind::Cycle {
                    self.step(p, -1, true);
                } else {
                    return vec![Act::Repaint];
                }
            }
            (Btn::Left, false) => match h.kind {
                Kind::Cycle if view::dropdown(p) => {
                    self.ui.popup = Some(p);
                    self.relayout();
                }
                Kind::Cycle => self.step(p, 1, true),
                Kind::Toggle => self.step(p, 1, true),
                Kind::Knob => return self.begin_drag(p, view::KNOB_PX, Axis::Up, None),
                Kind::Slider => {
                    let at = view::slider_norm(h.r, x);
                    return self.begin_drag(p, h.r.w - 12.0, Axis::Right, Some(at));
                }
                Kind::Bar => {
                    let at = view::bar_norm(h.r, y);
                    return self.begin_drag(p, h.r.h - 4.0, Axis::Up, Some(at));
                }
                Kind::Tab(_) | Kind::Sub(_) | Kind::Item(_) | Kind::Dismiss => {}
            },
        }
        vec![Act::Repaint]
    }

    /// Movement of a locked drag in logical px since the last event.
    pub fn delta(&mut self, dx: f32, dy: f32, m: Mods) -> Vec<Act> {
        let Some(Drag { param, v, px, axis }) = self.drag else {
            return Vec::new();
        };
        let fine = if m.ctrl { FINE } else { 1.0 };
        let moved = match axis {
            Axis::Up => -dy,
            Axis::Right => dx,
        };
        let v = (v + moved * fine / px).clamp(0.0, 1.0);
        self.vals.set(SPECS[param].id, v);
        self.drag = Some(Drag { param, v, px, axis });
        vec![Act::Repaint]
    }

    /// The press ends.
    pub fn up(&mut self) -> Vec<Act> {
        if let Some(drag) = self.drag.take() {
            self.vals.end(SPECS[drag.param].id);
        }
        self.pressed = None;
        vec![Act::Repaint]
    }

    /// The wheel over a control: `notches` up are positive.
    pub fn wheel(&mut self, x: f32, y: f32, notches: f32, m: Mods) -> Vec<Act> {
        let Some(h) = self.hit(x, y) else {
            return Vec::new();
        };
        if notches == 0.0 || !h.is_param() {
            return Vec::new();
        }
        let p = h.param;
        self.last = Some(p);
        let sign = notches.signum() as i32;
        match h.kind {
            Kind::Cycle | Kind::Toggle => self.step(p, sign, false),
            _ if view::is_int(p) => self.step(p, sign, false),
            _ => {
                let id = SPECS[p].id;
                let fine = if m.ctrl { FINE } else { 1.0 };
                let v = (self.vals.norm(id) + notches * WHEEL_STEP * fine).clamp(0.0, 1.0);
                self.vals.edit(id, v);
            }
        }
        vec![Act::Repaint]
    }
}

/// Rind's editor, as nih-plug's wrapper knows it.
pub struct RindEditor {
    params: Arc<RindParams>,
    #[cfg(feature = "embed")]
    scale: AtomicU32,
    /// The open editor's waker, for parameter changes.
    #[cfg(feature = "embed")]
    slot: os_plugkit::gui::Slot,
}

impl RindEditor {
    pub fn new(params: Arc<RindParams>) -> Self {
        RindEditor {
            params,
            #[cfg(feature = "embed")]
            scale: AtomicU32::new(1f32.to_bits()),
            #[cfg(feature = "embed")]
            slot: Default::default(),
        }
    }

    /// A parameter changed: the open editor looks again.
    fn notify(&self) {
        #[cfg(feature = "embed")]
        os_plugkit::gui::notify(&self.slot);
    }
}

impl Editor for RindEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        #[cfg(feature = "embed")]
        {
            let scale = f32::from_bits(self.scale.load(Ordering::Relaxed));
            match parent {
                // Honeydew gives the window as an X11 window id on every platform but macOS.
                #[cfg(not(target_os = "macos"))]
                ParentWindowHandle::X11Window(window) => Box::new(gui::open(
                    window,
                    scale,
                    self.params.clone(),
                    context,
                    &self.slot,
                )),
                // macOS sets no scale (sizes are points): the editor paints at its window's backing scale.
                #[cfg(target_os = "macos")]
                ParentWindowHandle::AppKitNsView(view) => {
                    let scale = os_plugkit::gui::backing_scale(view as usize);
                    Box::new(gui::open(
                        view as usize,
                        scale,
                        self.params.clone(),
                        context,
                        &self.slot,
                    ))
                }
                _ => {
                    let _ = (context, scale);
                    Box::new(())
                }
            }
        }
        #[cfg(not(feature = "embed"))]
        {
            let _ = (parent, context);
            Box::new(())
        }
    }

    fn size(&self) -> (u32, u32) {
        (view::W, view::H)
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        #[cfg(feature = "embed")]
        self.scale.store(_factor.to_bits(), Ordering::Relaxed);
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.notify();
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.notify();
    }

    fn param_values_changed(&self) {
        self.notify();
    }
}

#[cfg(feature = "embed")]
mod gui {
    use std::sync::Arc;

    use nih_plug::prelude::GuiContext;
    use os_plugkit::gui::{self as px, Button, Input, Window};
    use os_plugkit::vals::Vals;
    use os_ui::Canvas;

    use super::{Act, Btn, Logic, Mods};
    use crate::params::RindParams;
    use crate::view;

    /// The editor on the app thread.
    struct Ed {
        win: Window,
        canvas: Canvas,
        buf: Vec<u32>,
        scale: f32,
        size: (u32, u32),
        logic: Logic,
        dirty: bool,
        /// The fingerprint of the values as last painted: a tick repaints only when it moved.
        signature: u64,
    }

    /// Opens editor `parent` at `scale` device pixels per logical pixel.
    pub fn open(
        parent: px::Parent,
        scale: f32,
        params: Arc<RindParams>,
        ctx: Arc<dyn GuiContext>,
        slot: &px::Slot,
    ) -> Option<px::EditorThread> {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let size = (
            (view::W as f32 * scale).round() as u32,
            (view::H as f32 * scale).round() as u32,
        );
        px::spawn(parent, size, slot, move |win| {
            let Ok(canvas) = Canvas::new(size.0, size.1, scale) else {
                return;
            };
            let vals = Vals::new(&*params, params.live.clone(), Some(ctx));
            let signature = vals.signature();
            let ed = Ed {
                win,
                canvas,
                buf: Vec::new(),
                scale,
                size,
                logic: Logic::new(vals),
                dirty: true,
                signature,
            };
            ed.run();
        })
    }

    fn btn(b: Button) -> Btn {
        match b {
            Button::Left => Btn::Left,
            Button::Right => Btn::Right,
            Button::Middle => Btn::Middle,
        }
    }

    impl Ed {
        fn run(mut self) {
            while let Some(input) = self.win.next() {
                match input {
                    Input::Expose => self.dirty = true,
                    Input::Tick => self.poll(),
                    Input::Down {
                        x,
                        y,
                        btn: b,
                        dbl: _,
                    } => {
                        let acts =
                            self.logic
                                .down(x / self.scale, y / self.scale, btn(b), self.mods());
                        self.act(acts);
                    }
                    Input::Delta { dx, dy } => {
                        let acts = self
                            .logic
                            .delta(dx / self.scale, dy / self.scale, self.mods());
                        self.act(acts);
                    }
                    Input::Up { .. } => {
                        self.win.unlock();
                        let acts = self.logic.up();
                        self.act(acts);
                    }
                    Input::Wheel { x, y, notches } => {
                        let acts =
                            self.logic
                                .wheel(x / self.scale, y / self.scale, notches, self.mods());
                        self.act(acts);
                    }
                    Input::Move { .. } | Input::Leave | Input::Key { .. } | Input::Drop { .. } => {}
                }
                if self.dirty && self.win.idle() {
                    self.paint();
                }
            }
            os_plugkit::park!(self);
        }

        fn mods(&self) -> Mods {
            let (ctrl, _shift, alt) = self.win.mods();
            Mods { ctrl, alt }
        }

        /// Timer / parameter change: expire confirmed edits and repaint if a value moved.
        fn poll(&mut self) {
            let expired = self.logic.vals.expire();
            let signature = self.logic.vals.signature();
            if expired || signature != self.signature {
                self.signature = signature;
                self.dirty = true;
            }
        }

        fn act(&mut self, acts: Vec<Act>) {
            for a in acts {
                match a {
                    Act::Repaint => self.dirty = true,
                    Act::Lock => self.win.lock(),
                }
            }
        }

        fn paint(&mut self) {
            self.dirty = false;
            self.logic.paint(&mut self.canvas);
            self.signature = self.logic.vals.signature();
            self.buf.resize((self.size.0 * self.size.1) as usize, 0);
            self.canvas.copy_to_xrgb(&mut self.buf);
            self.win.blit(&self.buf, self.size.0, self.size.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;

    /// The parameters must outlive the logic: `Vals` holds raw pointers into them, so every test binds them.
    fn logic() -> (Logic, Arc<RindParams>) {
        let params = Arc::new(RindParams::default());
        let vals = Vals::new(&*params, params.live.clone(), None);
        let mut l = Logic::new(vals);
        paint(&mut l);
        (l, params)
    }

    fn paint(l: &mut Logic) -> Canvas {
        let mut c = Canvas::new(view::W, view::H, 1.0).unwrap();
        l.paint(&mut c);
        c
    }

    fn tap(l: &mut Logic, r: os_ui::Rect) {
        l.down(r.cx(), r.cy(), Btn::Left, Mods::default());
        l.up();
        paint(l);
    }

    /// Opens display tab `tab` and its sub page `sub`.
    fn open(l: &mut Logic, tab: usize, sub: usize) {
        let r = l.hits.iter().find(|h| h.kind == Kind::Tab(tab)).unwrap().r;
        tap(l, r);
        let r = l.hits.iter().find(|h| h.kind == Kind::Sub(sub)).unwrap().r;
        tap(l, r);
    }

    /// The centre of the control of parameter `p` on the current screen.
    fn at(l: &Logic, p: usize) -> (f32, f32) {
        let h = l
            .hits
            .iter()
            .find(|h| h.param == p && h.is_param())
            .unwrap_or_else(|| panic!("no control for {}", SPECS[p].id));
        (h.r.cx(), h.r.cy())
    }

    /// The name number `i` of the open list.
    fn item(l: &Logic, i: usize) -> os_ui::Rect {
        l.hits.iter().find(|h| h.kind == Kind::Item(i)).unwrap().r
    }

    fn live(params: &RindParams, p: usize) -> f32 {
        let mut snap = RindParams::defaults();
        params.snapshot(&mut snap);
        snap[p]
    }

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
    };

    #[test]
    fn every_screen_paints_something_and_the_tabs_switch_it() {
        let (mut l, _params) = logic();
        for tab in 0..view::TABS.len() {
            open(&mut l, tab, 0);
            let c = paint(&mut l);
            let mut px = vec![0u32; (view::W * view::H) as usize];
            c.copy_to_xrgb(&mut px);
            let distinct: std::collections::HashSet<_> = px.iter().copied().collect();
            assert!(distinct.len() > 8, "tab {tab} is blank");
            assert_eq!(l.ui.tab, tab);
        }
        open(&mut l, view::TAB_FX, 6);
        assert_eq!(l.ui.sub(), 6);
    }

    #[test]
    fn a_knob_drag_sounds_at_once_and_a_middle_click_resets_it() {
        let (mut l, params) = logic();
        let (x, y) = at(&l, ids::MASTER_VOL);
        assert_eq!(l.down(x, y, Btn::Left, NONE), [Act::Repaint, Act::Lock]);
        assert!(l.drag.is_some());
        // 30 px up of 150: a fifth of the range, from the default 0.7.
        l.delta(0.0, -30.0, NONE);
        l.up();
        assert!(l.drag.is_none());
        let heard = live(&params, ids::MASTER_VOL);
        assert!((heard - 0.9).abs() < 1e-3, "{heard}");
        l.down(x, y, Btn::Middle, NONE);
        assert!((live(&params, ids::MASTER_VOL) - 0.7).abs() < 1e-4);
        // Alt-click resets too.
        l.down(x, y, Btn::Left, NONE);
        l.delta(0.0, -30.0, NONE);
        l.up();
        l.down(x, y, Btn::Left, Mods { alt: true, ..NONE });
        assert!((live(&params, ids::MASTER_VOL) - 0.7).abs() < 1e-4);
    }

    #[test]
    fn a_fine_drag_moves_a_quarter_as_far() {
        let (mut l, params) = logic();
        let (x, y) = at(&l, ids::MASTER_VOL);
        l.down(x, y, Btn::Left, NONE);
        l.delta(0.0, -30.0, Mods { ctrl: true, ..NONE });
        l.up();
        assert!((live(&params, ids::MASTER_VOL) - 0.75).abs() < 1e-3);
    }

    #[test]
    fn a_short_selector_steps_on_a_tap_wraps_and_goes_back_on_a_right_click() {
        let (mut l, params) = logic();
        let (x, y) = at(&l, ids::MONO);
        assert_eq!(live(&params, ids::MONO), 0.0);
        l.down(x, y, Btn::Left, NONE);
        l.up();
        assert_eq!(live(&params, ids::MONO), 1.0);
        assert_eq!(l.ui.popup, None, "two names step, they do not open a list");
        l.down(x, y, Btn::Right, NONE);
        assert_eq!(live(&params, ids::MONO), 0.0);
        l.down(x, y, Btn::Right, NONE);
        assert_eq!(live(&params, ids::MONO), 1.0, "wraps below the first");
        l.down(x, y, Btn::Left, NONE);
        assert_eq!(live(&params, ids::MONO), 0.0, "wraps past the last");
    }

    #[test]
    fn a_long_selector_opens_a_list_and_a_name_picks_it() {
        let (mut l, params) = logic();
        let wave = ids::OSC_BASE + ids::osc::WAVE;
        let (x, y) = at(&l, wave);
        assert_eq!(live(&params, wave), 1.0);
        l.down(x, y, Btn::Left, NONE);
        l.up();
        paint(&mut l);
        assert_eq!(l.ui.popup, Some(wave));
        assert_eq!(live(&params, wave), 1.0, "opening changes nothing");
        // Every name is on the list.
        let names = ids::WAVE_NAMES.len();
        for i in 0..names {
            item(&l, i);
        }
        tap(&mut l, item(&l, 5));
        assert_eq!(l.ui.popup, None);
        assert_eq!(live(&params, wave), 5.0);
        assert_eq!(l.last, Some(wave));
        // The list is closed: the control is back and a press on it opens the list again.
        let (x, y) = at(&l, wave);
        l.down(x, y, Btn::Left, NONE);
        assert_eq!(l.ui.popup, Some(wave));
    }

    #[test]
    fn a_press_outside_the_list_or_a_right_click_closes_it_without_a_change() {
        let (mut l, params) = logic();
        let wave = ids::OSC_BASE + ids::osc::WAVE;
        let (x, y) = at(&l, wave);
        l.down(x, y, Btn::Left, NONE);
        paint(&mut l);
        // A press on the master volume's place (under the shade) closes the list; the knob is not touched.
        let before = live(&params, ids::MASTER_VOL);
        assert!(l.down(2.0, 2.0, Btn::Left, NONE).contains(&Act::Repaint));
        assert_eq!(l.ui.popup, None);
        assert!(l.drag.is_none());
        assert_eq!(live(&params, wave), 1.0);
        assert_eq!(live(&params, ids::MASTER_VOL), before);
        // A right click on a name closes it without picking.
        l.down(x, y, Btn::Left, NONE);
        paint(&mut l);
        let r = item(&l, 3);
        l.down(r.cx(), r.cy(), Btn::Right, NONE);
        assert_eq!(l.ui.popup, None);
        assert_eq!(live(&params, wave), 1.0);
        // The wheel does nothing under an open list.
        l.down(x, y, Btn::Left, NONE);
        paint(&mut l);
        assert!(l.wheel(x, y, 1.0, NONE).is_empty());
        assert_eq!(live(&params, wave), 1.0);
    }

    #[test]
    fn a_matrix_row_picks_its_source_and_target_from_lists_and_sets_the_amount() {
        let (mut l, params) = logic();
        // Slots 9-12 are the third sub page.
        open(&mut l, view::TAB_MATRIX, 2);
        let slot = 8;
        let src = ids::MAT_BASE + slot * ids::MAT_STRIDE + ids::mat::SRC;
        let dst = ids::MAT_BASE + slot * ids::MAT_STRIDE + ids::mat::DST;
        let amt = ids::MAT_BASE + slot * ids::MAT_STRIDE + ids::mat::AMT;
        let (x, y) = at(&l, src);
        l.down(x, y, Btn::Left, NONE);
        l.up();
        paint(&mut l);
        assert_eq!(l.ui.popup, Some(src));
        tap(&mut l, item(&l, ids::msrc::ENV_3));
        assert_eq!(live(&params, src), ids::msrc::ENV_3 as f32);
        // The target list holds every target, whichever is chosen.
        let (x, y) = at(&l, dst);
        l.down(x, y, Btn::Left, NONE);
        l.up();
        paint(&mut l);
        let last = ids::TARGET_NAMES.len() - 1;
        tap(&mut l, item(&l, last));
        assert_eq!(live(&params, dst), last as f32);
        // The amount slider takes a press, then a drag.
        let h = *l.hits.iter().find(|h| h.param == amt).unwrap();
        l.down(h.r.x + 1.0, h.r.cy(), Btn::Left, NONE);
        assert_eq!(live(&params, amt), -1.0);
        l.up();
        l.down(h.r.cx(), h.r.cy(), Btn::Left, NONE);
        assert!(live(&params, amt).abs() < 0.02);
        // Then a drag right over half the slider's travel: from the middle to the top.
        l.delta((h.r.w - 12.0) / 2.0, 0.0, NONE);
        l.up();
        assert!(
            (live(&params, amt) - 1.0).abs() < 0.05,
            "{}",
            live(&params, amt)
        );
    }

    #[test]
    fn a_switch_flips_and_the_wheel_holds_at_the_ends() {
        let (mut l, params) = logic();
        open(&mut l, view::TAB_ARP, 0);
        let (x, y) = at(&l, ids::ARP_ON);
        assert_eq!(live(&params, ids::ARP_ON), 0.0);
        l.down(x, y, Btn::Left, NONE);
        assert_eq!(live(&params, ids::ARP_ON), 1.0);
        l.down(x, y, Btn::Left, NONE);
        assert_eq!(live(&params, ids::ARP_ON), 0.0);
        let (mx, my) = at(&l, ids::ARP_MODE);
        l.wheel(mx, my, 1.0, NONE);
        assert_eq!(live(&params, ids::ARP_MODE), 1.0);
        l.wheel(mx, my, -5.0, NONE);
        l.wheel(mx, my, -5.0, NONE);
        assert_eq!(live(&params, ids::ARP_MODE), 0.0);
    }

    #[test]
    fn whole_number_knobs_step_by_the_wheel() {
        let (mut l, params) = logic();
        open(&mut l, view::TAB_ARP, 0);
        let (x, y) = at(&l, ids::ARP_OCT);
        l.wheel(x, y, 1.0, NONE);
        assert_eq!(live(&params, ids::ARP_OCT), 2.0);
        l.wheel(x, y, 1.0, NONE);
        assert_eq!(live(&params, ids::ARP_OCT), 3.0);
    }

    #[test]
    fn an_arp_bar_is_set_where_it_is_pressed() {
        let (mut l, params) = logic();
        open(&mut l, view::TAB_ARP, 0);
        let step = ids::ARP_STEP_BASE + 2;
        let h = *l.hits.iter().find(|h| h.param == step).unwrap();
        l.down(h.r.cx(), h.r.y + 2.0, Btn::Left, NONE);
        l.up();
        assert!((live(&params, step) - 1.0).abs() < 1e-4);
        l.down(h.r.cx(), h.r.bottom() - 2.0, Btn::Left, NONE);
        l.up();
        assert!(live(&params, step).abs() < 1e-4);
        // The transpose bar is bipolar: its middle is 0 semitones.
        let tr = *l
            .hits
            .iter()
            .find(|h| h.param == ids::ARP_TR_BASE + 2)
            .unwrap();
        l.down(tr.r.cx(), tr.r.cy(), Btn::Left, NONE);
        l.up();
        assert!(live(&params, ids::ARP_TR_BASE + 2).abs() <= 1.0);
        // A tie toggles.
        let (x, y) = at(&l, ids::ARP_HOLD_BASE + 2);
        l.down(x, y, Btn::Left, NONE);
        assert_eq!(live(&params, ids::ARP_HOLD_BASE + 2), 1.0);
        // The second sub page holds steps 9-16.
        open(&mut l, view::TAB_ARP, 1);
        at(&l, ids::ARP_STEP_BASE + 10);
    }

    #[test]
    fn a_press_off_every_control_does_nothing() {
        let (mut l, params) = logic();
        assert!(l.down(1.0, 100.0, Btn::Left, NONE).is_empty());
        assert!(l.delta(0.0, -10.0, NONE).is_empty());
        assert_eq!(live(&params, ids::MASTER_VOL), 0.7);
    }

    #[test]
    fn a_tab_change_keeps_each_tabs_sub_page() {
        let (mut l, _params) = logic();
        open(&mut l, view::TAB_FX, 3);
        open(&mut l, view::TAB_ARP, 0);
        open(&mut l, view::TAB_FX, 3);
        assert_eq!(l.ui.sub(), 3, "the effects tab remembers its effect");
    }
}
