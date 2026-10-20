//! The playlist / clip-editor split: the playlist, a gap, then the bottom clip editor under its colour band. The clip
//! editors (piano roll, drum, automation and audio editors) open under the playlist, not over it; the mixer still
//! replaces both.
//!
//! - The playlist gets the view's height × the split fraction f (the window has no top margin here). A 4 px
//!   `background` gap follows. The clip editor gets the rest, at least 3 U (the editor's floor wins on a short view):
//!   first a band t tall in the current channel's colour ([`band`]), then the editor itself.
//! - The editors may fill the window. A playlist shorter than its ruler is snapped to 0, with no gap under it: the
//!   band is then the coloured line at the window's top, and the app does not draw the playlist (nothing of it takes
//!   touches or the focus over the editor).
//! - Editors open fully expanded. Every opening from the playlist (or the mixer) goes to f = 0, the editor filling the
//!   view, whatever the ribbon left; a switch between editors keeps the split.
//! - f moves toward its target with the rack's law (half the way a frame at 60 fps), snapping within 1/height px.
//! - Opening an editor animates f from 1 to 0. Closing the editor puts f back to 1 at once.
//! - While f moves, the editor is laid out at its settled size and slides up under the playlist, shown (clipped) only
//!   in the rest of the view: so the piano roll centres its notes, and every editor its rows, for the size it keeps.
//! - While an editor shows, a 0.5 U shadow lies over the playlist's bottom edge, clipped to the playlist ([`shadow`]).
//!
//! The close ribbon on the band serves every bottom editor ([`ribbon`]): a tap closes the editor, a vertical drag
//! resizes the split.

use eframe::egui::{Color32, CornerRadius, Id, Painter, Rect, Sense, Ui, pos2, vec2};

use crate::icons::{IconId, draw_icon};
use crate::metrics::Metrics;
use crate::theme::Rgba;
use crate::ui::buttons::gradient;
use crate::ui::timeline::ruler_h;
use crate::ui::{UiCtx, widgets};

/// The clip editor's minimum height, in U.
const MIN_U: f32 = 3.0;
/// The gap between them, in device px.
const GAP_PX: f32 = 4.0;
/// The close ribbon: its size and bottom corners' radius, in U; how far its touch area reaches past it left, right and
/// below; its right edge's place, left of the logo (from the screen's right edge) or in from the editor's.
const RIBBON: [f32; 2] = [1.5, 1.3];
const RIBBON_R: f32 = 0.3;
const RIBBON_REACH: f32 = 0.3;
const RIBBON_LOGO: f32 = 4.4;
const RIBBON_EDGE: f32 = 1.8;
/// A ribbon drag's playlist floor, in U; the rubber band's give past either floor; a last move longer than this many
/// U is a flick, which counts ten times.
const PLAYLIST_MIN_U: f32 = 2.5;
const RUBBER: f32 = 0.1;
const FLICK_U: f32 = 0.25;
/// The shadow over the playlist's bottom edge: its height in U and its alpha at the edge.
const SHADOW_U: f32 = 0.5;
const SHADOW_ALPHA: f32 = 0.18;

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
    pub fn step(&mut self, editor: bool, height_px: f32, step: f32) -> bool {
        if !editor {
            *self = Self::default();
            return false;
        }
        self.f = crate::ui::rack::slide(self.f, self.target, height_px, step);
        self.f != self.target
    }

    /// This frame's rects in the view `view`: the playlist at f; the clip editor at the target's size, moved down to
    /// the rest of the view; and that rest, the part of the editor shown. The editor and its shown part start below
    /// the colour band, t tall at the top of the rest ([`band`]).
    pub fn layout(&self, view: Rect, m: &Metrics) -> (Rect, Rect, Rect) {
        let (playlist, shown) = split_rects(view, self.f, m);
        let settled = split_rects(view, self.target, m).1;
        let editor = settled.translate(vec2(0.0, shown.top() - settled.top()));
        let below = |r: Rect| r.with_min_y((r.top() + m.t()).min(r.bottom()));
        (playlist, below(editor), below(shown))
    }
}

/// The colour band over the editor that starts at `editor_top`: t tall, across `view`'s width, right above it, in
/// `color` (the current channel's, `menuFill` without one). Not interactive.
pub fn band(painter: &Painter, view: Rect, editor_top: f32, color: Color32, m: &Metrics) {
    let band = Rect::from_x_y_ranges(view.x_range(), editor_top - m.t()..=editor_top);
    painter.rect_filled(band, 0.0, color);
}

/// The shadow over the bottom edge of the `playlist` while an editor shows: black from α 0 to 0.18 down its last 0.5 U,
/// clipped to the playlist.
pub fn shadow(painter: &Painter, playlist: Rect, m: &Metrics) {
    if playlist.height() <= 0.0 {
        return;
    }
    let top = (playlist.bottom() - m.u(SHADOW_U)).max(playlist.top());
    let rect = Rect::from_x_y_ranges(playlist.x_range(), top..=playlist.bottom());
    let edge = Color32::from_black_alpha((SHADOW_ALPHA * 255.0).round() as u8);
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(playlist));
    gradient(&painter, rect, Color32::TRANSPARENT, edge);
}

/// A ribbon press, between frames: the playlist's height at the press, whether it has gone more than 0.5 U up or down
/// (a drag, no tap), the finger's height for the playlist (unclamped) and its last move, in points.
#[derive(Clone, Copy, Debug)]
struct Grab {
    from: f32,
    dragging: bool,
    raw: f32,
    last: f32,
}

/// The close ribbon of the bottom editors, under the colour band over the editor at `editor_top` in `view` (the editor
/// area, left of the rack), `screen_right` the window's right edge, in the channel colour `fill`:
/// - 1.5 U × 1.3 U from the band's bottom, its left edge at min(screen right − 4.4 U, view right − 1.8 U) (just left
///   of the logo at full width); square top corners and 0.3 U bottom ones; icon 87 (down chevron) in the best-contrasting
///   theme colour of the fill; hovered, the fill × 1.25. Its touch area reaches 0.3 U past it left, right and below.
/// - A tap (no vertical move over 0.5 U) returns true: the host closes the editor (the playlist alone, the clip kept,
///   no undo step).
/// - A vertical drag over 0.5 U moves the split: the playlist's height at the press plus the drag, with a rubber band
///   (× 0.1) below the 2.5 U playlist floor and past the 3 U editor floor. The last move counts ten times when over
///   0.25 U (a flick). Let go below the playlist floor the editor fills the view (f = 0); past the editor floor it
///   closes (returns true); else the split stays there.
pub fn ribbon(
    ui: &mut Ui,
    split: &mut Split,
    (view, editor_top): (Rect, f32),
    screen_right: f32,
    color: Rgba,
    cx: &UiCtx,
) -> bool {
    let (c, m) = (&cx.theme.colors, cx.m);
    let size = vec2(m.u(RIBBON[0]), m.u(RIBBON[1]));
    let left = (screen_right - m.u(RIBBON_LOGO)).min(view.right() - m.u(RIBBON_EDGE));
    let rect = Rect::from_min_size(pos2(m.floor_px(left), editor_top), size);
    let reach = m.u(RIBBON_REACH);
    let touch = Rect::from_min_max(rect.min - vec2(reach, 0.0), rect.max + vec2(reach, reach));
    let id = Id::new("editor_close");
    let response = crate::ui::focus::interact(ui, touch, id, Sense::click_and_drag());
    let h = view.height();
    let (gap, lo) = (GAP_PX / m.ppp(), m.u(PLAYLIST_MIN_U));
    let hi = (h - gap - m.u(MIN_U)).max(lo);
    let mut closed = false;
    let grab = ui.data(|d| d.get_temp::<Grab>(id));
    if response.drag_started() {
        let from = split_rects(view, split.f, m).0.height();
        let grab = Grab {
            from,
            dragging: false,
            raw: from,
            last: 0.0,
        };
        ui.data_mut(|d| d.insert_temp(id, grab));
    } else if let Some(mut grab) = grab {
        if response.dragged() {
            let total = response.total_drag_delta().unwrap_or_default().y;
            grab.dragging |= total.abs() > m.u(crate::ui::gesture::CALM_U);
            if grab.dragging && h > 0.0 {
                grab.last = grab.from + total - grab.raw;
                grab.raw = grab.from + total;
                let shown = rubber(grab.raw, lo, hi);
                *split = Split {
                    f: shown / h,
                    target: shown / h,
                };
            }
            ui.data_mut(|d| d.insert_temp(id, grab));
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<Grab>(id));
            if !grab.dragging {
                closed = true;
            } else if h > 0.0 {
                let flick = if grab.last.abs() > m.u(FLICK_U) {
                    9.0 * grab.last
                } else {
                    0.0
                };
                let end = grab.raw + flick;
                if end < lo {
                    *split = Split {
                        f: 0.0,
                        target: 0.0,
                    };
                } else if end > hi {
                    closed = true;
                } else {
                    *split = Split {
                        f: end / h,
                        target: end / h,
                    };
                }
            }
        }
    }
    closed |= response.clicked();

    let fill = color.to_color32();
    let fill = if widgets::hovered(ui, &response) {
        widgets::brighter(fill, 1.25)
    } else {
        fill
    };
    let r = m.u(RIBBON_R) as u8;
    let radius = CornerRadius {
        nw: 0,
        ne: 0,
        sw: r,
        se: r,
    };
    ui.painter().rect_filled(rect, radius, fill);
    let ink = c.best_contrasting(color, &c.contrast_candidates());
    crate::ui::tooltip::tip(ui, rect, "Close/Resize editor", Some("close"));
    draw_icon(
        ui.painter(),
        rect,
        IconId::ChevronDown,
        ink.to_color32(),
        1.0,
        m,
    );
    closed
}

/// The playlist height `raw` a drag asks for, shown with the rubber band past `lo` and `hi`.
fn rubber(raw: f32, lo: f32, hi: f32) -> f32 {
    if raw < lo {
        lo - (lo - raw) * RUBBER
    } else if raw > hi {
        hi + (raw - hi) * RUBBER
    } else {
        raw
    }
}

/// The playlist's and the clip editor's rects in the view `view` at split fraction `f` (module docs). The playlist's
/// bottom is on a whole device pixel. A playlist shorter than its ruler is none, with no gap: so nothing of it lies over
/// the editor.
pub fn split_rects(view: Rect, f: f32, m: &Metrics) -> (Rect, Rect) {
    let (h, gap) = (view.height(), GAP_PX / m.ppp());
    let top = (h * f).min(h - gap - m.u(MIN_U)).max(0.0);
    let top = if top < ruler_h(m) { 0.0 } else { top };
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
    use eframe::egui::{Event, PointerButton, Pos2, Rangef, pos2, vec2};

    use crate::metrics::DEFAULT_UI_SCALE;
    use crate::ui::offscreen::Offscreen;

    /// The rig's view at ppp 1 (U = 32): the content area above the transport, 1170 × 349.
    const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1170.0, 349.0));

    fn metrics(ppp: f32) -> Metrics {
        Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE)
    }

    /// At f = 0 the playlist is none and the editor fills the view; under its ruler the playlist is none; past it, a
    /// 4 px gap follows it and the editor takes the rest, at least 3 U.
    #[test]
    fn the_playlist_takes_f_then_a_4_px_gap_and_the_editor_the_rest_at_ppp_1_and_2() {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let (playlist, editor) = split_rects(VIEW, 0.0, &m);
            assert_eq!((playlist.height(), editor), (0.0, VIEW), "ppp {ppp}");
            let ruler = ruler_h(&m);
            let (playlist, editor) = split_rects(VIEW, (ruler - 1.0) / 349.0, &m);
            assert_eq!((playlist.height(), editor), (0.0, VIEW), "ppp {ppp}");
            let (playlist, editor) = split_rects(VIEW, (ruler + 1.0) / 349.0, &m);
            assert_eq!(playlist.height(), m.round_px(ruler + 1.0), "ppp {ppp}");
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
        let short = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(400.0, 160.0));
        let (playlist, editor) = split_rects(short, 1.0, &m);
        assert_eq!(editor.height(), 96.0);
        assert_eq!(playlist.height(), 160.0 - 96.0 - 4.0);
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), eframe::egui::vec2(400.0, 50.0));
        // No room for the playlist: the editor fills the view, with no gap.
        let (playlist, editor) = split_rects(tiny, 1.0, &m);
        assert_eq!((playlist.height(), editor.height()), (0.0, 50.0));
    }

    /// The ribbon under the band: a tap closes; a vertical drag moves the split by the finger, with a rubber band past
    /// the floors; let go below the 2.5 U playlist floor the editor fills the view, past the 3 U editor floor it closes,
    /// and a last move over 0.25 U counts ten times.
    #[test]
    fn the_ribbon_closes_on_a_tap_and_its_drag_resizes_with_a_rubber_band_and_flicks() {
        let m = metrics(1.0);
        let ribbon_at = |split: Split| {
            let top = split.layout(VIEW, &m).1.top();
            pos2(1170.0 - 4.4 * 32.0 + 24.0, top + 20.0)
        };
        // One gesture from `split`, through `ys` (finger y, from the press), on the rig: (closed, the split after).
        let run = |split: Split, ys: &[f32]| {
            let (mut rig, mut split, mut closed) = (Offscreen::new(), split, false);
            let from = ribbon_at(split);
            let mut frame = |rig: &mut Offscreen, split: &mut Split, events: Vec<Event>| {
                rig.frame(events, &mut |ui, cx| {
                    let top = split.layout(VIEW, &m).1.top();
                    let red = Rgba::rgb(1.0, 0.0, 0.0);
                    closed |= ribbon(ui, split, (VIEW, top), 1170.0, red, cx);
                });
            };
            let button = |pos: Pos2, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frame(&mut rig, &mut split, vec![Event::PointerMoved(from)]);
            frame(&mut rig, &mut split, vec![button(from, true)]);
            for &dy in ys {
                frame(
                    &mut rig,
                    &mut split,
                    vec![Event::PointerMoved(from + vec2(0.0, dy))],
                );
            }
            let end = from + vec2(0.0, ys.last().copied().unwrap_or(0.0));
            frame(&mut rig, &mut split, vec![button(end, false)]);
            (closed, split)
        };
        let at = |f: f32| Split { f, target: f };
        // A tap, and a press that wandered 12 points sideways or 15 down, closes.
        assert!(run(at(0.5), &[]).0);
        assert!(run(at(0.5), &[15.0]).0);
        // Down 120 points (slowly: no flick): the playlist grows by them; only 60 is within its floor, so it goes back.
        let slow = |to: usize| (1..=to).map(|i| 5.0 * i as f32).collect::<Vec<_>>();
        let (closed, split) = run(at(0.0), &slow(24));
        assert!(!closed);
        assert!((split.f * 349.0 - 120.0).abs() < 1e-3, "{split:?}");
        assert_eq!(run(at(0.0), &slow(12)), (false, at(0.0)));
        // From 100 points, up 70 slowly: under the 80-point floor the rubber band gives a tenth; let go there, the
        // editor fills the view.
        let up: Vec<f32> = (1..=14).map(|i| -5.0 * i as f32).collect();
        let (closed, split) = run(at(100.0 / 349.0), &up);
        assert_eq!((closed, split), (false, at(0.0)));
        // Down past the editor's 3 U floor (349 − 4 − 96 = 249 points): it closes.
        let down: Vec<f32> = (1..=40).map(|i| 5.0 * i as f32).collect();
        assert!(run(at(100.0 / 349.0), &down).0);
        // A flick: a last move of 10 points counts ten times, so 100 + 40 + 90 = 230 points.
        let (closed, split) = run(at(100.0 / 349.0), &[10.0, 20.0, 30.0, 40.0]);
        assert!(!closed);
        assert!((split.f * 349.0 - 230.0).abs() < 1e-3, "{split:?}");
    }

    #[test]
    fn opening_halves_the_way_to_the_target_each_frame_and_snaps_within_a_pixel_and_closing_snaps_back()
     {
        let mut split = Split::default();
        assert_eq!(split.f, 1.0);
        let mut seen = Vec::new();
        while split.step(true, 349.0, 0.5) {
            seen.push(split.f);
        }
        assert_eq!(seen[..3], [0.5, 0.25, 0.125]);
        // 0.5⁹ < 1/349 ≤ 0.5⁸: the ninth frame snaps.
        assert_eq!((seen.len(), split.f), (8, 0.0));
        assert!(!split.step(true, 349.0, 0.5));
        assert!(!split.step(false, 349.0, 0.5));
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
            split.step(false, 349.0, 0.5);
            let mut seen = Vec::new();
            while split.step(true, 349.0, 0.5) {
                seen.push(split.f);
            }
            assert_eq!(seen[..2], [0.5, 0.25]);
            // The editor and its shown part start under the t band at the view's top.
            let below = VIEW.with_min_y(m.t());
            let (playlist, editor, shown) = split.layout(VIEW, &m);
            assert_eq!((playlist.height(), editor, shown), (0.0, below, below));
        }
        let mut split = dragged;
        assert!(!split.step(true, 349.0, 0.5));
        assert_eq!(split, dragged);
    }

    #[test]
    fn while_it_opens_the_editor_keeps_its_settled_size_and_slides_up_under_the_shrinking_playlist()
    {
        for ppp in [1.0, 2.0] {
            let m = metrics(ppp);
            let mut split = Split::default();
            split.step(true, 349.0 * ppp, 0.5);
            let (playlist, editor, shown) = split.layout(VIEW, &m);
            // 174.5 to the device pixel.
            let half = (174.5 * ppp).round() / ppp;
            assert_eq!(playlist.bottom(), half, "ppp {ppp}");
            assert_eq!(
                shown.y_range(),
                Rangef::new(half + 4.0 / ppp + m.t(), 349.0),
                "ppp {ppp}"
            );
            let settled = VIEW.with_min_y(m.t());
            assert_eq!(editor.size(), settled.size(), "ppp {ppp}");
            assert_eq!(editor.top(), shown.top(), "ppp {ppp}");
            while split.step(true, 349.0 * ppp, 0.5) {}
            let (_, editor, shown) = split.layout(VIEW, &m);
            assert_eq!((editor, shown), (settled, settled), "ppp {ppp}");
        }
    }
}
