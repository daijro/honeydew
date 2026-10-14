//! The pitch and mod wheels left of the keys (placed by [`crate::ui::keyboard_settings::wheel_rects`]).
//!
//! Each is the generic vertical slider under its label line, `MOD` or `PITCH` in `text`, with the slider's drag law and
//! the double-tap reset. They show the current channel's values and report their moves; the app applies them
//! ([`Wheels`]): PITCH is channel param 4, the pitch wheel, and MOD param 8, MIDI CC 1.
//!
//! - The label line is the slider's own 0.75 U strip, painted in `text` (the slider draws the track and thumb in the
//!   channel colour).
//! - A long press opens only `Reset`: the CC link items stay disabled for the channel's own MIDI CC params, so they are
//!   left out (the CTRL menu still has `Add automation track`).
//! - Without a channel to play (the master alone), they are disabled, as the keys are.

use eframe::egui::{Align2, Color32, Id, Rect, Ui};

use crate::metrics::FontSize;
use crate::ui::UiCtx;
use crate::ui::widgets::{Control, ParamMenu, v_slider};

/// The channel params the wheels drive: the pitch wheel and MIDI CC 1.
pub const PITCH: i32 = 4;
pub const MOD: i32 = 8;
/// The label line's height, in U.
const LABEL_H: f32 = 0.75;

/// What the wheels did this frame: the values they set, in order (a PITCH release sends its last value, then 0.5),
/// and the param of the one pressed, which becomes the recent control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wheels {
    pub moves: Vec<(i32, f32)>,
    pub pressed: Option<i32>,
}

/// Draws `MOD` and `PITCH` in `rects` (left to right) at `values` (MOD's, PITCH's) in the channel colour `channel`, and
/// returns what they did. `enabled`: there is a channel to play.
pub fn wheels(
    ui: &mut Ui,
    cx: &UiCtx,
    rects: [Rect; 2],
    channel: Color32,
    values: [f32; 2],
    enabled: bool,
) -> Wheels {
    let m = cx.m;
    let text = cx.theme.colors.text.to_color32();
    let mut out = Wheels::default();
    for ((rect, mut v), (name, param, default)) in rects
        .into_iter()
        .zip(values)
        .zip([("MOD", MOD, 0.0), ("PITCH", PITCH, 0.5)])
    {
        let (label, body) = rect.split_top_bottom_at_y(rect.top() + m.u(LABEL_H));
        ui.painter().text(
            label.center(),
            Align2::CENTER_CENTER,
            name,
            m.font_id(FontSize::Pt8),
            text,
        );
        let control = Control {
            menu: ParamMenu::Unbound,
            disabled: !enabled,
            ..Control::new(Id::new(("wheel", name)), body, "", channel, default)
        };
        let response = v_slider(ui, cx, &control, &mut v, |_| String::new());
        if enabled && response.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed())
        {
            out.pressed = Some(param);
        }
        if response.changed() {
            out.moves.push((param, v));
        }
        // PITCH springs back: its value, then 0.5 at once (no animation). MOD stays.
        if param == PITCH && (response.drag_stopped() || response.clicked()) {
            if out.moves.last() != Some(&(param, v)) {
                out.moves.push((param, v));
            }
            out.moves.push((param, 0.5));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::theme_by_id;
    use crate::ui::offscreen::Offscreen;
    use eframe::egui::{Event, PointerButton, Pos2, Shape, pos2, vec2};

    /// The wheels 2 U wide and 5 U tall at the left of a 1170-point screen, under a 1.75 U bar.
    fn rects() -> [Rect; 2] {
        let top = 540.0 - 5.0 * 32.0 - 0.5 * 32.0;
        [0.0, 64.0].map(|x| Rect::from_min_size(pos2(x, top), vec2(64.0, 160.0)))
    }

    fn run(
        rig: &mut Offscreen,
        values: &mut [f32; 2],
        events: Vec<Event>,
    ) -> (Wheels, Vec<String>) {
        let mut out = Wheels::default();
        let shapes = rig.frame(events, &mut |ui, cx| {
            out = wheels(ui, cx, rects(), Color32::RED, *values, true);
        });
        for &(param, v) in &out.moves {
            values[usize::from(param == PITCH)] = v;
        }
        let texts = Offscreen::texts(&shapes).into_iter().map(|t| t.0).collect();
        (out, texts)
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn the_wheels_are_labelled_mod_and_pitch_in_text() {
        let mut rig = Offscreen::new();
        let ink = theme_by_id(0).colors.text.to_color32();
        let shapes = rig.frame(vec![], &mut |ui, cx| {
            wheels(ui, cx, rects(), Color32::RED, [0.0, 0.5], true);
        });
        let texts = Offscreen::texts(&shapes);
        assert_eq!(texts, [("MOD".into(), ink), ("PITCH".into(), ink)]);
        // Each label sits in the top 0.75 U of its wheel.
        let at = |name: &str| {
            shapes.iter().find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == name => {
                    Some(t.visual_bounding_rect().center())
                }
                _ => None,
            })
        };
        for (name, rect) in ["MOD", "PITCH"].into_iter().zip(rects()) {
            let c = at(name).unwrap();
            assert!(rect.contains(c) && c.y < rect.top() + 24.0, "{name} {c:?}");
        }
    }

    #[test]
    fn pitch_follows_the_drag_and_springs_to_the_middle_on_release_and_mod_stays() {
        let [mod_rect, pitch_rect] = rects();
        let mut rig = Offscreen::new();
        let mut values = [0.0, 0.5];
        // PITCH: pressed on its thumb (the middle) and dragged up 40 points.
        let thumb = pos2(
            pitch_rect.center().x,
            pitch_rect.top() + 24.0 + (160.0 - 24.0) / 2.0,
        );
        run(&mut rig, &mut values, vec![Event::PointerMoved(thumb)]);
        let (pressed, _) = run(&mut rig, &mut values, vec![button(thumb, true)]);
        assert_eq!(pressed.pressed, Some(PITCH));
        let up = thumb - vec2(0.0, 40.0);
        run(&mut rig, &mut values, vec![Event::PointerMoved(up)]);
        assert!(values[1] > 0.6, "{values:?}");
        // Let go: its value goes out again, then 0.5.
        let (out, _) = run(&mut rig, &mut values, vec![button(up, false)]);
        let last = out.moves.iter().rev().take(2).copied().collect::<Vec<_>>();
        assert_eq!(last[0], (PITCH, 0.5));
        assert_eq!(last[1].0, PITCH);
        assert!(last[1].1 > 0.6);
        assert_eq!(values[1], 0.5);
        // MOD: dragged up from its bottom, it keeps its value when let go.
        let low = pos2(mod_rect.center().x, mod_rect.bottom() - 16.0);
        run(&mut rig, &mut values, vec![Event::PointerMoved(low)]);
        let (pressed, _) = run(&mut rig, &mut values, vec![button(low, true)]);
        assert_eq!(pressed.pressed, Some(MOD));
        run(
            &mut rig,
            &mut values,
            vec![Event::PointerMoved(low - vec2(0.0, 50.0))],
        );
        let (out, _) = run(
            &mut rig,
            &mut values,
            vec![button(low - vec2(0.0, 50.0), false)],
        );
        assert!(out.moves.iter().all(|&(p, _)| p == MOD));
        assert!(values[0] > 0.3, "{values:?}");
        // A double tap resets MOD to 0.
        for _ in 0..2 {
            run(&mut rig, &mut values, vec![button(low, true)]);
            run(&mut rig, &mut values, vec![button(low, false)]);
        }
        assert_eq!(values[0], 0.0);
    }

    #[test]
    fn disabled_wheels_take_nothing() {
        let mut rig = Offscreen::new();
        let thumb = rects()[1].center();
        let mut out = Wheels::default();
        for events in [
            vec![Event::PointerMoved(thumb)],
            vec![button(thumb, true)],
            vec![Event::PointerMoved(thumb - vec2(0.0, 40.0))],
            vec![button(thumb - vec2(0.0, 40.0), false)],
        ] {
            rig.frame(events, &mut |ui, cx| {
                let o = wheels(ui, cx, rects(), Color32::RED, [0.0, 0.5], false);
                out.moves.extend(o.moves);
                out.pressed = out.pressed.or(o.pressed);
            });
        }
        assert_eq!(out, Wheels::default());
    }
}
