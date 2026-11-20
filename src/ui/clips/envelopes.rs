//! Two clip-local lanes. Coordinates always use content ticks, including offsets and loop repeats.
use super::*;
use crate::song::points::{self, Point, PointKey};
use crate::ui::automation::shape::{self, Shaping};

const COLORS: [Color32; 2] = [Color32::from_rgb(255, 213, 64), Color32::from_rgb(80, 222, 255)];

pub(super) fn lanes(area: Rect) -> [Rect; 2] {
    let top = area.top() + area.height() * 0.38;
    let middle = (top + area.bottom()) * 0.5;
    [Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), middle)),
     Rect::from_min_max(pos2(area.left(), middle), area.max)]
}
fn value_rect(lane: Rect, m: &Metrics) -> Rect {
    lane.shrink2(vec2(0.0, (lane.height() * 0.16).min(m.u(0.15))))
}
fn area(b: Rect, m: &Metrics) -> Rect {
    Rect::from_x_y_ranges(b.x_range(), b.top() + m.u(NAME_H)..=b.bottom() - CONTENT_END_T * m.t())
}
fn base(clip: &Clip, lane: usize) -> u16 {
    points::raw(clip.file.sample.as_ref().map_or(if lane == 0 { banaforge_audio::drums::DEFAULTS[0] } else { 0.5 }, |s| s.params[lane]))
}
fn value_at(pts: &[Point], base: u16, tick: f64) -> f32 {
    let n = pts.partition_point(|p| f64::from(p.tick) <= tick);
    if n == 0 { return f32::from(base) / 65535.0; }
    let a = pts[n - 1];
    let Some(b) = pts.get(n) else { return points::unit(a.value); };
    let t = ((tick - f64::from(a.tick)) / f64::from(b.tick - a.tick)) as f32;
    points::shape_value(a.shape, f64::from(points::unit(a.value)), f64::from(points::unit(b.value)), f64::from(t)) as f32
}
fn xy(tick: f64, value: f32, origin: f64, axis: TimeAxis, r: Rect) -> Pos2 {
    pos2(axis.x(origin + tick), r.bottom() - value.clamp(0.0, 1.0) * r.height())
}

/// Bounded by visible pixels, not clip duration or the number of quantized audio events.
pub(super) fn paint(painter: &Painter, clip: &Clip, area: Rect, axis: TimeAxis, m: &Metrics) {
    if area.height() < m.u(0.65) { return; }
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() { return; }
    let Some((origin, lp, all)) = repeats(clip, axis.tick(visible.left()), axis.tick(visible.right())) else { return; };
    let px = 1.0 / m.ppp();
    // Tiny repeated loops must not create unbounded point/line work.
    if lp < f64::from(px / axis.beat_w) * f64::from(PPQ) { return; }
    for (k, lane) in lanes(area).into_iter().enumerate() {
        let p = painter.with_clip_rect(visible.intersect(lane));
        p.rect_filled(lane, 0.0, Color32::from_rgba_unmultiplied(8, 14, 24, 220));
        p.hline(lane.x_range(), lane.top(), Stroke::new(px, COLORS[k].gamma_multiply(0.45)));
        let values = value_rect(lane, m);
        if k == 1 {
            p.hline(lane.x_range(), values.center().y, Stroke::new(px, COLORS[k].gamma_multiply(0.2)));
        }
        let pts = points::points(clip, PointKey::Envelope(k as i16));
        for repeat in all.clone() {
            let origin = origin + repeat as f64 * lp;
            let first = axis.tick(visible.left()).max(origin) - origin;
            let last = (axis.tick(visible.right()) - origin).min(lp);
            if last <= first { continue; }
            let step = f64::from((2.0 * px) / axis.beat_w) * f64::from(PPQ);
            // Include exact controls and duplicate ticks: vertical jumps must not be diagonally smeared.
            let n = ((last - first) / step).ceil().min(4096.0) as usize;
            let mut times: Vec<f64> = (0..=n).map(|i| (first + i as f64 * step).min(last)).collect();
            times.push(last);
            for point in pts.iter().filter(|p| f64::from(p.tick) >= first && f64::from(p.tick) <= last) {
                times.push(f64::from(point.tick));
            }
            times.sort_by(f64::total_cmp); times.dedup();
            let mut line = Vec::with_capacity(times.len() + pts.len());
            for tick in times {
                let i = pts.partition_point(|p| f64::from(p.tick) < tick);
                if i < pts.len() && f64::from(pts[i].tick) == tick {
                    if i > 0 {
                        let prev = pts[i - 1];
                        // Square is a hold followed by a true instantaneous edge.
                        if prev.shape.kind == points::ShapeKind::Square {
                            line.push(xy(tick, points::unit(prev.value), origin, axis, values));
                        }
                    } else if tick > 0.0 { line.push(xy(tick, points::unit(base(clip, k)), origin, axis, values)); }
                    for point in pts[i..].iter().take_while(|p| f64::from(p.tick) == tick) {
                        line.push(xy(tick, points::unit(point.value), origin, axis, values));
                    }
                } else { line.push(xy(tick, value_at(&pts, base(clip, k), tick), origin, axis, values)); }
            }
            p.add(egui::Shape::line(line, Stroke::new(1.5 * px, COLORS[k])));
            for point in pts.iter().filter(|p| f64::from(p.tick) >= first && f64::from(p.tick) <= last) {
                let at = xy(f64::from(point.tick), points::unit(point.value), origin, axis, values);
                p.circle_filled(at, m.u(0.10).min(values.height() * 0.3).max(px * 2.0), Color32::from_rgb(8, 14, 24));
                p.circle_stroke(at, m.u(0.10).min(values.height() * 0.3).max(px * 2.0), Stroke::new(px, COLORS[k]));
            }
        }
        let label = Rect::from_min_size(pos2(visible.left(), lane.top()), vec2(m.u(1.0), lane.height()));
        p.rect_filled(label, 0.0, Color32::from_rgba_unmultiplied(8, 14, 24, 235));
        p.text(label.center(), Align2::CENTER_CENTER, if k == 0 { "VOL" } else { "PAN" }, m.font_id(FontSize::Pt8), COLORS[k]);
        if k == 1 && visible.width() > m.u(3.0) {
            p.text(pos2(visible.right()-m.u(0.1), lane.center().y), Align2::RIGHT_CENTER, "↑R · C · ↓L", m.font_id(FontSize::Pt8), COLORS[k].gamma_multiply(0.7));
        }
    }
}

struct Drag {
    clip: ClipRef, lane: usize, index: usize, origin: f64,
    original: Vec<Point>, expected: Vec<Point>, since: f64, press: Pos2, moved: bool, bound: bool,
}
struct Menu { clip: ClipRef, lane: usize, index: usize, frame: Rect, original: Vec<Point>, shaping: Shaping }
#[derive(Default)]
pub(super) struct Editor { drag: Option<Drag>, menu: Option<Menu>, owned: bool }
impl Editor {
    pub fn busy(&self) -> bool { self.drag.is_some() || self.menu.is_some() || self.owned }
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut AppCtx, blocks: &[(ClipRef, Rect)], selected: Option<ClipRef>, grid: Rect, axis: TimeAxis, snap: Option<f64>) -> bool {
        self.owned = false;
        if let Some(mut menu) = self.menu.take() {
            let Some(clip) = find(app.song, menu.clip) else { return false; };
            let key = PointKey::Envelope(menu.lane as i16);
            let own = menu.original.get(menu.index).map(|p| p.shape);
            let mut selection = vec![false; menu.original.len()];
            if let Some(s) = selection.get_mut(menu.index) { *s = true; }
            let close = shape::popup(ui.ctx(), &app.cx, Id::new("playlist-envelope-shape"), menu.frame, (&mut menu.shaping, own), COLORS[menu.lane]).close;
            let new = menu.shaping.on(&menu.original, &selection);
            if points::points(clip, key) != new { set(app, menu.clip, key, new, false); }
            if close { app.song_edited(true); } else { self.menu = Some(menu); }
            return true;
        }
        let (pos, pressed, released, down, secondary, cancel) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.secondary_clicked(), !i.focused || crate::ui::focus_lost(i) || i.multi_touch().is_some() || i.key_pressed(egui::Key::Escape)));
        if let Some(mut drag) = self.drag.take() {
            let key = PointKey::Envelope(drag.lane as i16);
            let valid = find(app.song, drag.clip).is_some_and(|c| points::points(c, key) == drag.expected);
            if !valid { return true; } // An external undo/delete wins; never overwrite it.
            if cancel || app.covered {
                set(app, drag.clip, key, drag.original, true);
                self.owned = true; return true;
            }
            if let Some(p) = pos {
                drag.moved |= p.distance(drag.press) > app.cx.m.u(0.15);
                if drag.bound && !drag.moved && ui.input(|i| i.time) - drag.since > 0.5 && down {
                    self.menu = Some(Menu { clip: drag.clip, lane: drag.lane, index: drag.index,
                        frame: Rect::from_center_size(p, vec2(20.0,20.0)), original:drag.original, shaping:Shaping::default() });
                    return true;
                }
                if (drag.moved || !drag.bound) && let Some((_, b)) = blocks.iter().find(|(r,_)| *r == drag.clip) {
                    let values = value_rect(lanes(area(clip_box(*b, app.cx.m), app.cx.m))[drag.lane], app.cx.m);
                    let tick = axis.tick(p.x) - drag.origin;
                    let tick = snap.map_or(tick, |s| (tick / s).round() * s).round().max(0.0);
                    let end = find(app.song, drag.clip).and_then(Clip::loop_ticks).unwrap_or(1.0);
                    let point = &mut drag.expected[drag.index];
                    point.tick = tick.min((end - 1.0).max(0.0)) as u32;
                    point.value = points::raw(((values.bottom() - p.y) / values.height()).clamp(0.0, 1.0));
                    let mut tagged: Vec<_> = drag.expected.iter().copied().enumerate().collect();
                    tagged.sort_by_key(|(_,p)| p.tick);
                    drag.index = tagged.iter().position(|(i,_)| *i == drag.index).unwrap();
                    drag.expected = tagged.into_iter().map(|(_,p)| p).collect();
                    set(app, drag.clip, key, drag.expected.clone(), released || !down);
                }
            }
            if down && !released { self.drag = Some(drag); } else { app.song_edited(true); }
            self.owned = true; return true;
        }
        if cancel || app.covered || !(pressed || secondary) { return false; }
        let Some(p) = pos.filter(|p| grid.contains(*p)) else { return false; };
        // Same topmost ordering as clip bodies. Unselected clips keep ordinary select/move behavior.
        let Some(&(r, b)) = blocks.iter().rev().find(|(_,b)| b.contains(p)) else { return false; };
        if Some(r) != selected { return false; }
        let Some(clip) = find(app.song, r) else { return false; };
        if clip.file.sample.is_none() || app.song.channels[r.channel].lanes[r.lane].collapsed { return false; }
        let a = area(clip_box(b, app.cx.m), app.cx.m);
        if a.height() < app.cx.m.u(0.65) { return false; }
        let Some((lane, lane_rect)) = lanes(a).into_iter().enumerate().find(|(_,v)| v.contains(p)) else { return false; };
        let key = PointKey::Envelope(lane as i16);
        let original = points::points(clip, key);
        let Some(lp) = clip.loop_ticks() else { return false; };
        let start = f64::from(clip.start_tick) - clip.offset_beats * f64::from(PPQ);
        let origin = start + ((axis.tick(p.x) - start) / lp).floor() * lp;
        let values = value_rect(lane_rect, app.cx.m);
        let closest = original.iter().enumerate().map(|(i,point)| (i, xy(f64::from(point.tick), points::unit(point.value), origin, axis, values).distance(p)))
            .filter(|(_,d)| *d < app.cx.m.u(0.4)).min_by(|a,b| a.1.total_cmp(&b.1)).map(|v| v.0);
        let mut pts = original.clone();
        let index = closest.unwrap_or_else(|| {
            let tick = axis.tick(p.x) - origin;
            let tick = snap.map_or(tick, |s| (tick / s).round() * s).round().clamp(0.0, (lp-1.0).max(0.0)) as u32;
            points::insert(&mut pts, Point { tick, value: points::raw(((values.bottom()-p.y)/values.height()).clamp(0.0,1.0)), ..Default::default() })
        });
        if secondary {
            if closest.is_some() { self.menu = Some(Menu { clip:r, lane, index, frame:Rect::from_center_size(p, vec2(20.0,20.0)), original, shaping:Shaping::default() }); }
            return true;
        }
        set(app, r, key, pts.clone(), false);
        self.drag = Some(Drag { clip:r, lane, index, origin, original, expected:pts, since:ui.input(|i| i.time), press:p, moved:false, bound:closest.is_some() });
        true
    }
}
fn set(app: &mut AppCtx, r: ClipRef, key: PointKey, pts: Vec<Point>, final_edit: bool) {
    if audio_editor::apply(app.song, (r.channel,r.lane,r.id), &Action::Points { key, points:pts }) {
        if final_edit { app.song_edited(true); }
        else { app.edit = app.edit.max(Some(crate::ui::views::SongEdit::Audition)); }
    }
}
