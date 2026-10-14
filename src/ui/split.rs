//! The playlist / clip-editor split: the playlist, the splitter, then the bottom clip editor. The clip editors (piano
//! roll, drum, automation and audio editors) open under the playlist, not over it; the mixer still replaces both.
//!
//! - The playlist gets the view's height × the split fraction f (the window has no top margin here). A 4 px gap
//!   follows. The clip editor gets the rest, at least 3 U (the editor's floor wins on a short view).
//! - The editors may fill the window. A playlist shorter than its 1.5 U ruler is snapped to 0, with no gap under it:
//!   the editor starts at the view's top, and the app does not draw the playlist (nothing of it takes touches or the
//!   focus over the editor).
//! - Editors open fully expanded. Every opening from the playlist (or the mixer) goes to f = 0, the editor filling the
//!   view, whatever the splitter left; a switch between editors keeps the split.
//! - f moves toward its target with the rack's law, f ← f + 0.5·(target − f) per frame, snapping within 1/height px.
//! - Opening an editor animates f from 1 to 0. Closing the editor puts f back to 1 at once.
//! - While f moves, the editor is laid out at its settled size and slides up under the playlist, shown (clipped) only
//!   in the rest of the view: so the piano roll centres its notes, and every editor its rows, for the size it keeps.
//! - The splitter is the gap grown 0.25 U up and down (with the playlist at 0, the view's top edge). Dragging it moves
//!   the split with the finger and sets the target, which holds until the editor closes.
//!
//! The one close X of the bottom editors is here too ([`close_button`]): a tap puts the split at 1 at once (the app
//! shows the playlist alone; the editor keeps its clip).

use eframe::egui::{Id, Rangef, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::Metrics;
use crate::ui::UiCtx;
use crate::ui::buttons::filled_circle_button;
use crate::ui::timeline::RULER_H;

/// The clip editor's minimum height, in U.
const MIN_U: f32 = 3.0;
/// The gap between them, in device px.
const GAP_PX: f32 = 4.0;
/// The splitter's touch zone reaches this far past the gap each way, in U.
const GRAB_U: f32 = 0.25;
/// The shared close X's disc, in U.
const CLOSE_U: f32 = 1.5;

/// The split fraction `f` and its `target`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    pub f: f32,
    pub target: f32,
}

/// Closed: the playlist alone, the next opening going to 0 (module docs).
impl Default for Split {
    fn default() -> Self {
        Self {
            f: 1.0,
            target: 0.0,
        }
    }
}

impl Split {
    /// One frame of the split while a clip `editor` shows or not, in a view `height_px` device pixels tall. Returns
    /// whether it is still moving. With none it is the default, so the next opening goes to 0 (editors open fully
    /// expanded).
    pub fn step(&mut self, editor: bool, height_px: f32) -> bool {
        if !editor {
            *self = Self::default();
            return false;
        }
        self.f = crate::ui::rack::slide(self.f, self.target, height_px);
        self.f != self.target
    }

    /// This frame's rects in the view `view`: the playlist at f; the clip editor at the target's size, moved down to
    /// the rest of the view; and that rest, the part of the editor shown.
    pub fn layout(&self, view: Rect, m: &Metrics) -> (Rect, Rect, Rect) {
        let (playlist, shown) = split_rects(view, self.f, m);
        let settled = split_rects(view, self.target, m).1;
        let editor = settled.translate(vec2(0.0, shown.top() - settled.top()));
        (playlist, editor, shown)
    }

    /// The splitter under the playlist `playlist` of the view `view`, for this frame's drag (module docs). Register it
    /// after the playlist and the editor, so that it takes the touches in its zone.
    pub fn splitter(&mut self, ui: &mut Ui, view: Rect, playlist: Rect, m: &Metrics) {
        let gap = Rangef::new(playlist.bottom(), playlist.bottom() + GAP_PX / m.ppp());
        let zone = Rect::from_x_y_ranges(view.x_range(), gap.expand(m.u(GRAB_U)));
        let id = Id::new("editor_split");
        let response = ui.interact(zone, id, Sense::drag());
        // From the playlist's height at the press, by the whole drag: a playlist snapped to 0 (`split_rects`) still
        // follows a slow finger back down.
        if response.drag_started() {
            ui.data_mut(|d| d.insert_temp(id, playlist.height()));
        }
        let from = ui.data(|d| d.get_temp::<f32>(id));
        if let (Some(from), Some(delta)) = (from, response.total_drag_delta())
            && view.height() > 0.0
        {
            let f = ((from + delta.y) / view.height()).clamp(0.0, 1.0);
            *self = Self { f, target: f };
        }
    }
}

/// The bottom editors' one close X: a 1.5 U `menuFill` disc with icon 19 in `text` (scale 1). Its square's left edge is
/// at trunc(R − 3.5 U) while the rack tab lies on the column's right edge R = `view.right()` (`rack_closed`), else at
/// trunc(R − 1.5 U − 5 px); its top is centred on the 4 px divider above `editor_top`, editor top − (4 px + 1.5 U)/2,
/// never above the window top (`view.top()`). Draw it after the splitter, which it lies over. Returns whether it was
/// tapped.
pub fn close_button(
    ui: &mut Ui,
    view: Rect,
    editor_top: f32,
    rack_closed: bool,
    cx: &UiCtx,
) -> bool {
    let (c, m) = (&cx.theme.colors, cx.m);
    let (d, px) = (m.u(CLOSE_U), 1.0 / m.ppp());
    let left = if rack_closed {
        view.right() - m.u(3.5)
    } else {
        view.right() - d - 5.0 * px
    };
    let top = (editor_top - (GAP_PX * px + d) / 2.0).max(view.top());
    let square = Rect::from_min_size(pos2(m.floor_px(left), top), Vec2::splat(d));
    let id = Id::new("editor_close");
    let x = filled_circle_button(ui, id, square.center(), d, c.menu_fill.to_color32());
    draw_icon(
        ui.painter(),
        x.rect(),
        IconId::Close,
        c.text.to_color32(),
        1.0,
        m,
    );
    x.clicked()
}

/// The playlist's and the clip editor's rects in the view `view` at split fraction `f` (module docs). The playlist's
/// bottom is on a whole device pixel. A playlist shorter than its ruler is none, with no gap: so nothing of it lies over
/// the editor.
pub fn split_rects(view: Rect, f: f32, m: &Metrics) -> (Rect, Rect) {
    let (h, gap) = (view.height(), GAP_PX / m.ppp());
    let top = (h * f).min(h - gap - m.u(MIN_U)).max(0.0);
    let top = if top < m.u(RULER_H) { 0.0 } else { top };
    let y = m.round_px(view.top() + top);
    let gap = if y > view.top() { gap } else { 0.0 };
    (
        view.with_max_y(y),
        view.with_min_y((y + gap).min(view.bottom())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, pos2, vec2};

    use crate::metrics::DEFAULT_UI_SCALE_INDEX;
    use crate::ui::offscreen::Offscreen;

    /// The rig's view at ppp 1 (U = 32): the content area above the transport, 1170 × 349.
    const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1170.0, 349.0));

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE_INDEX)
    }

    /// At f = 0 the playlist is none and the editor fills the view; under its 1.5 U ruler the playlist is none; past
    /// it, a 4 px gap follows it and the editor takes the rest, at least 3 U.
    #[test]
    fn the_playlist_takes_f_then_a_4_px_gap_and_the_editor_the_rest_at_ppp_1_and_2() {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let (playlist, editor) = split_rects(VIEW, 0.0, &m);
            assert_eq!((playlist.height(), editor), (0.0, VIEW), "ppp {ppp}");
            let ruler = m.u(1.5);
            let (playlist, editor) = split_rects(VIEW, (ruler - 1.0) / 349.0, &m);
            assert_eq!((playlist.height(), editor), (0.0, VIEW), "ppp {ppp}");
            let (playlist, editor) = split_rects(VIEW, (ruler + 1.0) / 349.0, &m);
            assert_eq!(playlist.height(), ruler + 1.0, "ppp {ppp}");
            assert_eq!(editor.top() - playlist.bottom(), 4.0 / ppp, "ppp {ppp}");
            assert_eq!(editor.bottom(), VIEW.bottom(), "ppp {ppp}");
            assert_eq!(
                (playlist.x_range(), editor.x_range()),
                (VIEW.x_range(), VIEW.x_range())
            );
            // Half way: h·f, on a whole device pixel.
            let (playlist, _) = split_rects(VIEW, 0.5, &m);
            assert_eq!(playlist.bottom(), (174.5 * ppp).round() / ppp, "ppp {ppp}");
            // f = 1: the editor keeps its 3 U.
            let (playlist, editor) = split_rects(VIEW, 1.0, &m);
            assert_eq!(editor.height(), 3.0 * m.u(1.0), "ppp {ppp}");
            assert_eq!(playlist.bottom() + 4.0 / ppp, editor.top(), "ppp {ppp}");
        }
    }

    #[test]
    fn on_a_short_view_the_editor_keeps_its_3_u_and_the_playlist_gives_way() {
        let m = metrics(1.0);
        let short = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(400.0, 150.0));
        let (playlist, editor) = split_rects(short, 1.0, &m);
        assert_eq!(editor.height(), 96.0);
        assert_eq!(playlist.height(), 150.0 - 96.0 - 4.0);
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(400.0, 50.0));
        // No room for the playlist: the editor fills the view, with no gap.
        let (playlist, editor) = split_rects(tiny, 1.0, &m);
        assert_eq!((playlist.height(), editor.height()), (0.0, 50.0));
    }

    #[test]
    fn with_the_editor_at_the_view_top_the_close_x_keeps_to_the_window_top() {
        let m = metrics(1.0);
        let short = Rect::from_min_size(pos2(0.0, 10.0), vec2(400.0, 100.0));
        let (_, editor) = split_rects(short, 0.0, &m);
        assert_eq!(editor.top(), 10.0);
        let mut rig = Offscreen::new();
        let mut square = None;
        for _ in 0..2 {
            rig.frame(vec![], &mut |ui, cx| {
                close_button(ui, short, editor.top(), true, cx);
                square = ui
                    .ctx()
                    .read_response(Id::new("editor_close"))
                    .map(|r| r.rect);
            });
        }
        // Centred on the divider it would start 26 points above the editor, out of the view; it starts at the view's top.
        assert_eq!(square.map(|r| r.top()), Some(short.top()));
    }

    #[test]
    fn opening_halves_the_way_to_the_target_each_frame_and_snaps_within_a_pixel_and_closing_snaps_back()
     {
        let mut split = Split::default();
        assert_eq!(split.f, 1.0);
        let mut seen = Vec::new();
        while split.step(true, 349.0) {
            seen.push(split.f);
        }
        assert_eq!(seen[..3], [0.5, 0.25, 0.125]);
        // 0.5⁹ < 1/349 ≤ 0.5⁸: the ninth frame snaps.
        assert_eq!((seen.len(), split.f), (8, 0.0));
        assert!(!split.step(true, 349.0));
        assert!(!split.step(false, 349.0));
        assert_eq!(split.f, 1.0);
    }

    /// Editors open fully expanded. Every opening from closed goes to f = 0, the playlist none and the editor the whole
    /// view, whatever the splitter left; a switch between editors (open throughout) keeps the split.
    #[test]
    fn every_opening_from_closed_fills_the_view_and_a_switch_keeps_the_split() {
        let m = metrics(1.0);
        let dragged = Split {
            f: 0.4,
            target: 0.4,
        };
        for mut split in [Split::default(), dragged] {
            split.step(false, 349.0);
            let mut seen = Vec::new();
            while split.step(true, 349.0) {
                seen.push(split.f);
            }
            assert_eq!(seen[..2], [0.5, 0.25]);
            let (playlist, editor, shown) = split.layout(VIEW, &m);
            assert_eq!((playlist.height(), editor, shown), (0.0, VIEW, VIEW));
        }
        let mut split = dragged;
        assert!(!split.step(true, 349.0));
        assert_eq!(split, dragged);
    }

    #[test]
    fn while_it_opens_the_editor_keeps_its_settled_size_and_slides_up_under_the_shrinking_playlist()
    {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let mut split = Split::default();
            split.step(true, 349.0 * ppp);
            let (playlist, editor, shown) = split.layout(VIEW, &m);
            // 174.5 to the device pixel.
            let half = (174.5 * ppp).round() / ppp;
            assert_eq!(playlist.bottom(), half, "ppp {ppp}");
            assert_eq!(
                shown.y_range(),
                Rangef::new(half + 4.0 / ppp, 349.0),
                "ppp {ppp}"
            );
            let settled = split_rects(VIEW, 0.0, &m).1;
            assert_eq!(editor.size(), settled.size(), "ppp {ppp}");
            assert_eq!(editor.top(), shown.top(), "ppp {ppp}");
            while split.step(true, 349.0 * ppp) {}
            let (_, editor, shown) = split.layout(VIEW, &m);
            assert_eq!((editor, shown), (settled, settled), "ppp {ppp}");
        }
    }

    #[test]
    fn a_drag_on_the_gap_moves_the_split_and_its_target() {
        let m = metrics(1.0);
        let mut split = Split {
            f: 0.0,
            target: 0.0,
        };
        let mut rig = Offscreen::new();
        let run = |rig: &mut Offscreen, split: &mut Split, events: Vec<Event>| {
            rig.frame(events, &mut |ui, _| {
                let (playlist, _) = split_rects(VIEW, split.f, &m);
                split.splitter(ui, VIEW, playlist, &m);
            });
        };
        // The playlist is 0: the gap is 0..4 on the view's top edge; the zone reaches 8 points past it each way.
        let from = pos2(600.0, 2.0);
        let button = |pos: Pos2, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        run(&mut rig, &mut split, vec![Event::PointerMoved(from)]);
        run(&mut rig, &mut split, vec![button(from, true)]);
        for y in [22.0, 42.0, 82.0] {
            run(
                &mut rig,
                &mut split,
                vec![Event::PointerMoved(pos2(600.0, y))],
            );
        }
        run(&mut rig, &mut split, vec![button(pos2(600.0, 82.0), false)]);
        // The playlist's bottom followed the finger: 0 + 80.
        assert_eq!(
            split,
            Split {
                f: 80.0 / 349.0,
                target: 80.0 / 349.0,
            }
        );
        assert_eq!(split_rects(VIEW, split.f, &m).0.bottom(), 80.0);
        // Outside the zone a drag does nothing.
        let before = split;
        let far = pos2(600.0, 200.0);
        run(&mut rig, &mut split, vec![button(far, true)]);
        run(
            &mut rig,
            &mut split,
            vec![Event::PointerMoved(pos2(600.0, 260.0))],
        );
        run(
            &mut rig,
            &mut split,
            vec![button(pos2(600.0, 260.0), false)],
        );
        assert_eq!(split, before);
    }
}
