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

use crate::params::{CHOICE_MAX, RindParams};
use crate::view::{self, Hit, Kind};

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

/// A drag: `v` is the unrounded normalised value it has reached, over `px` for the whole range.
#[derive(Clone, Copy, Debug)]
struct Drag {
    id: &'static str,
    v: f32,
    px: f32,
}

/// The editor's logic.
pub struct Logic {
    pub vals: Vals,
    hits: Vec<Hit>,
    drag: Option<Drag>,
    pressed: Option<&'static str>,
}

/// A fine drag or wheel step (Ctrl) is this share of a normal one.
const FINE: f32 = 0.25;
/// A wheel notch moves a knob or fader this much of its range.
const WHEEL_STEP: f32 = 0.02;

impl Logic {
    pub fn new(vals: Vals) -> Self {
        Logic {
            vals,
            hits: Vec::new(),
            drag: None,
            pressed: None,
        }
    }

    /// Paints the editor and keeps its hit regions.
    pub fn paint(&mut self, c: &mut Canvas) {
        let hits = {
            let vals = &self.vals;
            view::paint(c, &|id| vals.norm(id), self.pressed)
        };
        self.hits = hits;
    }

    fn hit(&self, x: f32, y: f32) -> Option<(&'static str, Kind)> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.r.contains(x, y))
            .map(|h| (h.id, h.kind))
    }

    fn reset(&mut self, id: &str) {
        let default = self.vals.default_norm(id);
        self.vals.edit(id, default);
    }

    fn begin_drag(&mut self, id: &'static str, px: f32) -> Vec<Act> {
        self.vals.begin(id);
        self.drag = Some(Drag {
            id,
            v: self.vals.norm(id),
            px,
        });
        self.pressed = Some(id);
        vec![Act::Repaint, Act::Lock]
    }

    /// A press at (`x`, `y`) in logical px.
    pub fn down(&mut self, x: f32, y: f32, btn: Btn, m: Mods) -> Vec<Act> {
        let Some((id, kind)) = self.hit(x, y) else {
            return Vec::new();
        };
        match (kind, btn) {
            (_, Btn::Middle) => self.reset(id),
            (_, Btn::Left) if m.alt => self.reset(id),
            (Kind::Choice(value), Btn::Left) => {
                let norm = self.vals.plain_to_norm(id, value as f32);
                self.vals.edit(id, norm);
            }
            (Kind::Knob, Btn::Left) => return self.begin_drag(id, view::KNOB_PX),
            (Kind::Fader, Btn::Left) => return self.begin_drag(id, view::FADER_PX),
            (_, Btn::Right) => return Vec::new(),
        }
        vec![Act::Repaint]
    }

    /// Movement of a locked drag in logical px since the last event: up raises the value.
    pub fn delta(&mut self, dy: f32, m: Mods) -> Vec<Act> {
        let Some(Drag { id, v, px }) = self.drag else {
            return Vec::new();
        };
        let fine = if m.ctrl { FINE } else { 1.0 };
        let v = (v - dy * fine / px).clamp(0.0, 1.0);
        self.vals.set(id, v);
        self.drag = Some(Drag { id, v, px });
        vec![Act::Repaint]
    }

    /// The press ends.
    pub fn up(&mut self) -> Vec<Act> {
        if let Some(drag) = self.drag.take() {
            self.vals.end(drag.id);
        }
        self.pressed = None;
        vec![Act::Repaint]
    }

    /// The wheel over a control: `notches` up are positive.
    pub fn wheel(&mut self, x: f32, y: f32, notches: f32, m: Mods) -> Vec<Act> {
        let Some((id, kind)) = self.hit(x, y) else {
            return Vec::new();
        };
        if notches == 0.0 {
            return Vec::new();
        }
        match kind {
            Kind::Choice(_) => {
                let to =
                    (self.vals.plain(id).round() + notches.signum()).clamp(0.0, CHOICE_MAX as f32);
                let norm = self.vals.plain_to_norm(id, to);
                self.vals.edit(id, norm);
            }
            Kind::Knob | Kind::Fader => {
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
                    Input::Delta { dx: _, dy } => {
                        let acts = self.logic.delta(dy / self.scale, self.mods());
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
    use crate::params::{CHOICE, FADER, KNOB};

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

    fn knob() -> (f32, f32) {
        view::knob_center()
    }

    #[test]
    fn painting_draws_every_control_and_the_regions_do_not_overlap() {
        let (l, _) = logic();
        // The knob, the fader and the choice's four cells.
        assert_eq!(l.hits.len(), 6);
        for (i, a) in l.hits.iter().enumerate() {
            for b in &l.hits[i + 1..] {
                assert!(a.r.intersect(&b.r).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
        let mut l = l;
        let c = paint(&mut l);
        let mut px = vec![0u32; (view::W * view::H) as usize];
        c.copy_to_xrgb(&mut px);
        assert!(px.iter().any(|&p| p != px[0]), "the frame is blank");
    }

    #[test]
    fn a_knob_drag_sounds_at_once_and_a_middle_click_resets_it() {
        let (mut l, params) = logic();
        let (x, y) = knob();
        assert_eq!(
            l.down(x, y, Btn::Left, Mods::default()),
            [Act::Repaint, Act::Lock]
        );
        assert!(l.drag.is_some());
        // 30 px up of the knob's whole-range drag: a fifth of the range.
        l.delta(-30.0, Mods::default());
        l.up();
        assert!(l.drag.is_none());
        let heard = params.live.value(&params.knob);
        assert!((heard - 0.7).abs() < 1e-4, "{heard}");
        // The middle button (a double tap on Honeydew's card) puts it back.
        l.down(x, y, Btn::Middle, Mods::default());
        assert_eq!(params.live.value(&params.knob), 0.5);
        // So does Alt-click.
        l.down(x, y, Btn::Left, Mods::default());
        l.delta(-30.0, Mods::default());
        l.up();
        l.down(
            x,
            y,
            Btn::Left,
            Mods {
                alt: true,
                ..Mods::default()
            },
        );
        assert_eq!(params.live.value(&params.knob), 0.5);
    }

    #[test]
    fn a_fine_drag_moves_a_quarter_as_far() {
        let (mut l, params) = logic();
        let (x, y) = knob();
        l.down(x, y, Btn::Left, Mods::default());
        l.delta(
            -30.0,
            Mods {
                ctrl: true,
                ..Mods::default()
            },
        );
        l.up();
        let heard = params.live.value(&params.knob);
        assert!((heard - 0.55).abs() < 1e-4, "{heard}");
    }

    #[test]
    fn the_fader_drags_through_its_skewed_range_and_resets() {
        let (mut l, params) = logic();
        let r = view::fader_rect();
        l.down(r.cx(), r.cy(), Btn::Left, Mods::default());
        l.delta(60.0, Mods::default());
        l.up();
        // Half the range down from the default (1.0): a normalised 0.5, which the skew maps to another plain value.
        let heard = params.live.value(&params.fader);
        assert!(heard > 0.0 && heard < 1.0, "{heard}");
        l.down(r.cx(), r.cy(), Btn::Middle, Mods::default());
        assert_eq!(params.live.value(&params.fader), 1.0);
    }

    #[test]
    fn a_choice_cell_picks_its_value_and_the_wheel_steps_it() {
        let (mut l, params) = logic();
        let r = view::choice_rect(2);
        l.down(r.cx(), r.cy(), Btn::Left, Mods::default());
        assert_eq!(params.live.value(&params.choice), 2);
        l.wheel(r.cx(), r.cy(), 1.0, Mods::default());
        assert_eq!(params.live.value(&params.choice), 3);
        l.wheel(r.cx(), r.cy(), 1.0, Mods::default());
        assert_eq!(
            params.live.value(&params.choice),
            3,
            "held at the last value"
        );
        l.wheel(r.cx(), r.cy(), -2.0, Mods::default());
        assert_eq!(params.live.value(&params.choice), 2);
    }

    #[test]
    fn a_press_off_every_control_does_nothing() {
        let (mut l, params) = logic();
        assert!(l.down(2.0, 2.0, Btn::Left, Mods::default()).is_empty());
        assert!(l.delta(-10.0, Mods::default()).is_empty());
        assert_eq!(params.live.value(&params.knob), 0.5);
    }

    #[test]
    fn the_regions_name_the_parameters() {
        let (l, _) = logic();
        let ids: Vec<_> = l.hits.iter().map(|h| h.id).collect();
        assert_eq!(ids, [KNOB, FADER, CHOICE, CHOICE, CHOICE, CHOICE]);
    }
}
