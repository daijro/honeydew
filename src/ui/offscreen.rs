//! Test rig for the transport components: each call runs one egui frame in an offscreen context, with synthetic
//! pointer input and its own clock. `U` = 32 points (ppp 1).

use eframe::egui::{
    self, CentralPanel, Context, Event, Frame, Modifiers, PointerButton, Pos2, RawInput, Rect, Ui,
    Vec2, epaint::ClippedShape, pos2,
};

use crate::metrics::{DEFAULT_UI_SCALE, Metrics};
use crate::theme::theme_by_id;
use crate::ui::UiCtx;

pub const SCREEN: Rect = Rect {
    min: pos2(0.0, 0.0),
    max: pos2(1170.0, 540.0),
};

pub fn metrics() -> Metrics {
    Metrics::from_pixels_per_point(1.0, DEFAULT_UI_SCALE)
}

pub struct Offscreen {
    pub ctx: Context,
    pub time: f64,
    pub modifiers: Modifiers,
}

impl Offscreen {
    pub fn new() -> Self {
        Self {
            ctx: Context::default(),
            time: 0.0,
            modifiers: Modifiers::NONE,
        }
    }

    /// One frame, 1/60 s after the last; `f` paints. Returns what was painted.
    pub fn frame(
        &mut self,
        events: Vec<Event>,
        f: &mut dyn FnMut(&mut Ui, &UiCtx),
    ) -> Vec<ClippedShape> {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(SCREEN),
            time: Some(self.time),
            modifiers: self.modifiers,
            events,
            ..Default::default()
        };
        let (theme, m) = (theme_by_id(0), metrics());
        let cx = UiCtx {
            theme: &theme,
            m: &m,
        };
        let output = self.ctx.run(raw, |ctx| {
            CentralPanel::default()
                .frame(Frame::NONE)
                .show(ctx, |ui| f(ui, &cx));
        });
        output.shapes
    }

    fn button(&mut self, pos: Pos2, pressed: bool, f: &mut dyn FnMut(&mut Ui, &UiCtx)) {
        let event = Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: self.modifiers,
        };
        self.frame(vec![event], f);
    }

    /// Moves to `pos`, presses and releases, one frame each.
    pub fn click(&mut self, pos: Pos2, f: &mut dyn FnMut(&mut Ui, &UiCtx)) {
        self.frame(vec![Event::PointerMoved(pos)], f);
        self.button(pos, true, f);
        self.button(pos, false, f);
    }

    /// Presses at `from`, moves through `offsets` (from the press point), one frame each, and releases.
    pub fn drag(&mut self, from: Pos2, offsets: &[Vec2], f: &mut dyn FnMut(&mut Ui, &UiCtx)) {
        self.frame(vec![Event::PointerMoved(from)], f);
        self.button(from, true, f);
        for &d in offsets {
            self.frame(vec![Event::PointerMoved(from + d)], f);
        }
        let end = from + offsets.last().copied().unwrap_or_default();
        self.button(end, false, f);
    }

    /// The texts painted in `shapes`, with their colours.
    pub fn texts(shapes: &[ClippedShape]) -> Vec<(String, egui::Color32)> {
        shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.job.sections[0].format.color,
                )),
                _ => None,
            })
            .collect()
    }
}

/// A press of `key` with `modifiers` and its release, for one frame. A second press without a release in between would
/// be an auto-repeat, which the keymap's actions ignore (`Keymap::consume_press`).
pub fn key_tap(key: egui::Key, modifiers: Modifiers) -> Vec<Event> {
    [true, false]
        .map(|pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .to_vec()
}
