//! Two clip-local lanes. Coordinates always use content ticks, including offsets and loop repeats.
use super::*;
use crate::song::points::{self, Point, PointKey};
use crate::ui::automation::shape::{self, Shaping};

/// Scan existing events without allocation; untouched clips never prepare/tessellate curves.
fn used(clip: &Clip, lane: usize) -> bool {
    let default = base(clip, lane);
    clip.events.iter().any(|event| matches!(event,
        Event::Envelope { env_param, value, control: true, .. }
        if *env_param == lane as i16 && points::raw(*value) != default))
}

pub(super) fn lanes(area: Rect) -> [Rect; 2] {
    let top = if area.height() >= 64.0 {
        area.top() + area.height() * 0.38
    } else {
        area.top()
    };
    let middle = (top + area.bottom()) * 0.5;
    [
        Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), middle)),
        Rect::from_min_max(pos2(area.left(), middle), area.max),
    ]
}
fn value_rect(lane: Rect, m: &Metrics) -> Rect {
    lane.shrink2(vec2(0.0, (lane.height() * 0.16).min(m.u(0.15))))
}
fn area(b: Rect, m: &Metrics) -> Rect {
    Rect::from_x_y_ranges(
        b.x_range(),
        b.top() + m.u(NAME_H)..=b.bottom() - CONTENT_END_T * m.t(),
    )
}
fn base(clip: &Clip, lane: usize) -> u16 {
    points::raw(clip.file.sample.as_ref().map_or(
        if lane == 0 {
            banaforge_audio::drums::DEFAULTS[0]
        } else {
            0.5
        },
        |s| s.params[lane],
    ))
}
thread_local! {
    static CURVES: std::cell::RefCell<std::collections::VecDeque<(Vec<Point>, std::sync::Arc<Vec<(u32, f32)>>)>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}
/// Identical quantized control-to-event law to playback; cached separately from immutable PCM peaks.
fn prepared(pts: &[Point]) -> std::sync::Arc<Vec<(u32, f32)>> {
    use banaforge_audio::automation::{AutomationClip, AutomationPoint};
    CURVES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(i) = cache.iter().position(|(p, _)| p == pts) {
            let hit = cache.remove(i).unwrap();
            let out = hit.1.clone();
            cache.push_back(hit);
            return out;
        }
        let built = AutomationClip::new(
            0,
            1,
            1,
            0,
            1,
            pts.iter()
                .map(|p| AutomationPoint {
                    tick: p.tick.into(),
                    value: p.value,
                    shape: p.shape,
                })
                .collect(),
        )
        .unwrap();
        let steps = std::sync::Arc::new(
            built
                .events()
                .iter()
                .map(|e| (e.tick as u32, e.normalized_value()))
                .collect::<Vec<_>>(),
        );
        let bytes = |p: &[Point], s: &[(u32, f32)]| {
            p.len() * std::mem::size_of::<Point>() + std::mem::size_of_val(s)
        };
        let size = bytes(pts, &steps);
        if size <= 1024 * 1024 {
            while cache.len() >= 64
                || cache.iter().map(|(p, s)| bytes(p, s)).sum::<usize>() + size > 1024 * 1024
            {
                cache.pop_front();
            }
            cache.push_back((pts.to_vec(), steps.clone()));
        }
        steps
    })
}
fn value_at(steps: &[(u32, f32)], base: u16, tick: f64) -> f32 {
    let n = steps.partition_point(|(t, _)| f64::from(*t) <= tick);
    n.checked_sub(1).map_or(points::unit(base), |i| steps[i].1)
}
fn xy(tick: f64, value: f32, origin: f64, axis: TimeAxis, r: Rect) -> Pos2 {
    pos2(
        axis.x(origin + tick),
        r.bottom() - value.clamp(0.0, 1.0) * r.height(),
    )
}

/// Bounded by visible pixels, not clip duration or the number of quantized audio events.
pub(super) fn paint(painter: &Painter, clip: &Clip, area: Rect, axis: TimeAxis, m: &Metrics) {
    let display = crate::ui::envelope_display::EnvelopeDisplay::current(painter.ctx());
    if !display.visible || (!used(clip, 0) && !used(clip, 1)) { return; }
    let colors = [display.color(0), display.color(1)];
    if area.height() < m.u(0.65) {
        return;
    }
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let Some((origin, lp, all)) =
        repeats(clip, axis.tick(visible.left()), axis.tick(visible.right()))
    else {
        return;
    };
    let px = 1.0 / m.ppp();
    // Tiny repeated loops must not create unbounded point/line work.
    if lp < f64::from(px / axis.beat_w) * f64::from(PPQ) {
        return;
    }
    for (k, lane) in lanes(area).into_iter().enumerate() {
        if !used(clip, k) { continue; }
        let p = painter.with_clip_rect(visible.intersect(lane));
        let values = value_rect(lane, m);
        if k == 1 {
            p.hline(
                lane.x_range(),
                values.center().y,
                Stroke::new(px, colors[k].gamma_multiply(0.2)),
            );
        }
        let pts = points::points(clip, PointKey::Envelope(k as i16));
        let steps = prepared(&pts);
        for repeat in all.clone() {
            let origin = origin + repeat as f64 * lp;
            let first = axis.tick(visible.left()).max(origin) - origin;
            let last = (axis.tick(visible.right()) - origin).min(lp);
            if last <= first {
                continue;
            }
            let step = f64::from((2.0 * px) / axis.beat_w) * f64::from(PPQ);
            // Include exact controls and duplicate ticks: vertical jumps must not be diagonally smeared.
            let n = ((last - first) / step).ceil().min(4096.0) as usize;
            let mut times: Vec<f64> = (0..=n)
                .map(|i| (first + i as f64 * step).min(last))
                .collect();
            times.push(last);
            for point in pts
                .iter()
                .filter(|p| f64::from(p.tick) >= first && f64::from(p.tick) <= last)
            {
                times.push(f64::from(point.tick));
            }
            times.sort_by(f64::total_cmp);
            times.dedup();
            // Add the engine's rounded step times for Square/Stairs and duplicate-tick edges.
            let mut jumps = Vec::new();
            for pair in pts.windows(2) {
                if pair[0].tick == pair[1].tick {
                    jumps.push(f64::from(pair[1].tick));
                }
                if let Some(risers) = pair[0].shape.risers() {
                    for x in risers {
                        jumps.push(
                            (f64::from(pair[0].tick) + x * f64::from(pair[1].tick - pair[0].tick))
                                .round(),
                        );
                    }
                }
            }
            if let Some(p) = pts.first().filter(|p| p.tick > 0) {
                jumps.push(f64::from(p.tick));
            }
            times.extend(jumps.iter().copied().filter(|t| *t >= first && *t <= last));
            times.sort_by(f64::total_cmp);
            times.dedup();
            jumps.sort_by(f64::total_cmp);
            jumps.dedup();
            let mut line = Vec::with_capacity(times.len() + jumps.len());
            for tick in times {
                if jumps.binary_search_by(|v| v.total_cmp(&tick)).is_ok() {
                    line.push(xy(
                        tick,
                        value_at(&steps, base(clip, k), tick - 0.01),
                        origin,
                        axis,
                        values,
                    ));
                }
                line.push(xy(
                    tick,
                    value_at(&steps, base(clip, k), tick),
                    origin,
                    axis,
                    values,
                ));
            }
            p.add(egui::Shape::line(line, Stroke::new(1.5 * px, colors[k])));
            for point in pts
                .iter()
                .filter(|p| f64::from(p.tick) >= first && f64::from(p.tick) <= last)
            {
                let at = xy(
                    f64::from(point.tick),
                    points::unit(point.value),
                    origin,
                    axis,
                    values,
                );
                p.circle_filled(
                    at,
                    m.u(0.10).min(values.height() * 0.3).max(px * 2.0),
                    Color32::from_rgb(8, 14, 24),
                );
                p.circle_stroke(
                    at,
                    m.u(0.10).min(values.height() * 0.3).max(px * 2.0),
                    Stroke::new(px, colors[k]),
                );
            }
        }

    }
}

struct Drag {
    clip: ClipRef,
    lane: usize,
    index: usize,
    origin: f64,
    original: Vec<Point>,
    expected: Vec<Point>,
    since: f64,
    press: Pos2,
    moved: bool,
    bound: bool,
}
struct Menu {
    clip: ClipRef,
    lane: usize,
    index: usize,
    frame: Rect,
    original: Vec<Point>,
    expected: Vec<Point>,
    shaping: Shaping,
}
#[derive(Default)]
pub(super) struct Editor {
    drag: Option<Drag>,
    menu: Option<Menu>,
    owned: bool,
    editing: Option<ClipRef>,
}
impl Editor {
    pub fn finish(&mut self) -> bool {
        let a = self.drag.take().is_some();
        let b = self.menu.take().is_some();
        let active = a || b;
        self.owned = false;
        active
    }
    pub fn status(&self, ui: &egui::Ui, cx: &UiCtx, b: Rect, r: ClipRef, grid: Rect) {
        let display = crate::ui::envelope_display::EnvelopeDisplay::current(ui.ctx());
        if !display.visible { return; }
        let a = area(b, cx.m);
        if a.height() < cx.m.u(0.65) {
            return;
        }
        for (k, lane) in lanes(a).into_iter().enumerate() {
            let label = Rect::from_min_size(
                pos2(b.left().max(grid.left()), lane.top()),
                vec2(cx.m.u(1.0), lane.height()),
            );
            if self.editing == Some(r) {
                ui.painter_at(grid).rect_stroke(
                    lane,
                    0.0,
                    Stroke::new(1.0 / cx.m.ppp(), display.color(k)),
                    StrokeKind::Inside,
                );
            }
            crate::ui::tooltip::tip(
                ui,
                label,
                "Double-tap a selected audio clip to arm both envelope lanes (double-tap again to exit). Yellow = volume; cyan = pan by default. Drag a point; tap empty lane space to add. Hold or right-click a point for curve shape and tension. PAN: top = right, middle = center, bottom = left. Clip header/waveform still move the clip.",
                None,
            );
        }
    }
    pub fn busy(&self) -> bool {
        self.drag.is_some() || self.menu.is_some() || self.owned
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut AppCtx,
        blocks: &[(ClipRef, Rect)],
        selected: Option<ClipRef>,
        grid: Rect,
        axis: TimeAxis,
        snap: Option<f64>,
    ) -> bool {
        self.owned = false;
        if let Some(mut menu) = self.menu.take() {
            let Some(clip) = find(app.song, menu.clip) else {
                return false;
            };
            let key = PointKey::Envelope(menu.lane as i16);
            if points::points(clip, key) != menu.expected {
                return true;
            }
            let own = menu.original.get(menu.index).map(|p| p.shape);
            let mut selection = vec![false; menu.original.len()];
            if let Some(s) = selection.get_mut(menu.index) {
                *s = true;
            }
            let close = shape::popup(
                ui.ctx(),
                &app.cx,
                Id::new("playlist-envelope-shape"),
                menu.frame,
                (&mut menu.shaping, own),
                crate::ui::envelope_display::EnvelopeDisplay::current(ui.ctx()).color(menu.lane),
            )
            .close;
            let new = menu.shaping.on(&menu.original, &selection);
            if points::points(clip, key) != new {
                set(app, menu.clip, key, new.clone(), false);
            }
            menu.expected = new;
            if close {
                app.song_edited(true);
            } else {
                self.menu = Some(menu);
            }
            return true;
        }
        let (pos, pressed, released, down, secondary, cancel) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.secondary_clicked(),
                !i.focused
                    || crate::ui::focus_lost(i)
                    || i.multi_touch().is_some()
                    || i.key_pressed(egui::Key::Escape),
            )
        });
        if let Some(mut drag) = self.drag.take() {
            ui.ctx().request_repaint();
            let key = PointKey::Envelope(drag.lane as i16);
            let valid =
                find(app.song, drag.clip).is_some_and(|c| points::points(c, key) == drag.expected);
            if !valid {
                return true;
            } // An external undo/delete wins; never overwrite it.
            if cancel || app.covered {
                set(app, drag.clip, key, drag.original, true);
                app.song_edited(true);
                self.owned = true;
                return true;
            }
            if let Some(p) = pos {
                drag.moved |= p.distance(drag.press) > app.cx.m.u(0.15);
                if drag.bound && !drag.moved && ui.input(|i| i.time) - drag.since > 0.5 && down {
                    self.menu = Some(Menu {
                        clip: drag.clip,
                        lane: drag.lane,
                        index: drag.index,
                        frame: Rect::from_center_size(p, vec2(20.0, 20.0)),
                        expected: drag.original.clone(),
                        original: drag.original,
                        shaping: Shaping::default(),
                    });
                    return true;
                }
                if (drag.moved || !drag.bound)
                    && let Some((_, b)) = blocks.iter().find(|(r, _)| *r == drag.clip)
                {
                    let values = value_rect(
                        lanes(area(clip_box(*b, app.cx.m), app.cx.m))[drag.lane],
                        app.cx.m,
                    );
                    let tick = axis.tick(p.x) - drag.origin;
                    let tick = snap
                        .map_or(tick, |s| (tick / s).round() * s)
                        .round()
                        .max(0.0);
                    let end = find(app.song, drag.clip)
                        .and_then(Clip::loop_ticks)
                        .unwrap_or(1.0);
                    let point = &mut drag.expected[drag.index];
                    point.tick = tick.min((end - 1.0).max(0.0)) as u32;
                    point.value =
                        points::raw(((values.bottom() - p.y) / values.height()).clamp(0.0, 1.0));
                    let mut tagged: Vec<_> = drag.expected.iter().copied().enumerate().collect();
                    tagged.sort_by_key(|(_, p)| p.tick);
                    drag.index = tagged.iter().position(|(i, _)| *i == drag.index).unwrap();
                    drag.expected = tagged.into_iter().map(|(_, p)| p).collect();
                    set(
                        app,
                        drag.clip,
                        key,
                        drag.expected.clone(),
                        released || !down,
                    );
                }
            }
            if down && !released {
                self.drag = Some(drag);
            } else {
                app.song_edited(true);
            }
            self.owned = true;
            return true;
        }
        if !crate::ui::envelope_display::EnvelopeDisplay::current(ui.ctx()).visible { return false; }
        if cancel || app.covered || !(pressed || secondary) {
            return false;
        }
        let Some(p) = pos.filter(|p| grid.contains(*p)) else {
            return false;
        };
        // Same topmost ordering as clip bodies. Unselected clips keep ordinary select/move behavior.
        let Some(&(r, b)) = blocks.iter().rev().find(|(_, b)| b.contains(p)) else {
            return false;
        };
        if Some(r) != selected {
            return false;
        }
        let Some(clip) = find(app.song, r) else {
            return false;
        };
        if clip.file.sample.is_none() || app.song.channels[r.channel].lanes[r.lane].collapsed {
            return false;
        }
        let a = area(clip_box(b, app.cx.m), app.cx.m);
        if a.height() < app.cx.m.u(0.65) {
            return false;
        }
        let Some((lane, lane_rect)) = lanes(a)
            .into_iter()
            .enumerate()
            .find(|(_, v)| v.contains(p))
        else {
            return false;
        };
        // Double-tap a selected clip to arm point creation without covering its waveform.
        if ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) {
            self.editing = if self.editing == Some(r) {
                None
            } else {
                Some(r)
            };
            self.owned = true;
            return true;
        }
        let key = PointKey::Envelope(lane as i16);
        let original = points::points(clip, key);
        let Some(lp) = clip.loop_ticks() else {
            return false;
        };
        let start = f64::from(clip.start_tick) - clip.offset_beats * f64::from(PPQ);
        let origin = start + ((axis.tick(p.x) - start) / lp).floor() * lp;
        let values = value_rect(lane_rect, app.cx.m);
        let closest = original
            .iter()
            .enumerate()
            .map(|(i, point)| {
                (
                    i,
                    xy(
                        f64::from(point.tick),
                        points::unit(point.value),
                        origin,
                        axis,
                        values,
                    )
                    .distance(p),
                )
            })
            .filter(|(_, d)| *d < app.cx.m.u(0.4))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|v| v.0);
        if self.editing != Some(r) && closest.is_none() {
            return false;
        }
        let mut pts = original.clone();
        let index = closest.unwrap_or_else(|| {
            let tick = axis.tick(p.x) - origin;
            let tick = snap
                .map_or(tick, |s| (tick / s).round() * s)
                .round()
                .clamp(0.0, (lp - 1.0).max(0.0)) as u32;
            points::insert(
                &mut pts,
                Point {
                    tick,
                    value: points::raw(((values.bottom() - p.y) / values.height()).clamp(0.0, 1.0)),
                    ..Default::default()
                },
            )
        });
        if secondary {
            if closest.is_some() {
                self.menu = Some(Menu {
                    clip: r,
                    lane,
                    index,
                    frame: Rect::from_center_size(p, vec2(20.0, 20.0)),
                    expected: original.clone(),
                    original,
                    shaping: Shaping::default(),
                });
            }
            return true;
        }
        set(app, r, key, pts.clone(), false);
        ui.ctx().request_repaint();
        self.drag = Some(Drag {
            clip: r,
            lane,
            index,
            origin,
            original,
            expected: pts,
            since: ui.input(|i| i.time),
            press: p,
            moved: false,
            bound: closest.is_some(),
        });
        true
    }
}
fn set(app: &mut AppCtx, r: ClipRef, key: PointKey, pts: Vec<Point>, final_edit: bool) {
    if audio_editor::apply(
        app.song,
        (r.channel, r.lane, r.id),
        &Action::Points { key, points: pts },
    ) {
        if final_edit {
            app.song_edited(true);
        } else {
            app.edit = app.edit.max(Some(crate::ui::views::SongEdit::Audition));
        }
    }
}

#[cfg(test)]
mod clip_envelope_tests {
    use super::*;
    use crate::audio_link::AudioLink;
    use crate::project_files::mem::MemFiles;
    use crate::settings::Settings;
    use crate::song::points::{Shape, ShapeKind};
    use crate::ui::main_screen::Regions;
    use crate::ui::offscreen::Offscreen;
    const BLOCK: Rect = Rect {
        min: pos2(20.0, 50.0),
        max: pos2(660.0, 190.0),
    };
    fn empty_clip() -> Clip {
        let mut lane = Lane::new(LaneKind::Audio);
        lane.add_clip(0, 4.0).clone()
    }
    fn axis() -> TimeAxis {
        TimeAxis {
            origin: 20.0,
            beat_w: 160.0,
        }
    }
    struct Rig {
        off: Offscreen,
        editor: Editor,
        song: Song,
        edits: Vec<Option<crate::ui::views::SongEdit>>,
        selected: Option<ClipRef>,
    }
    impl Rig {
        fn new() -> Self {
            let mut song = Song::default();
            song.add_channel(ChannelKind::AudioClip);
            let clip = song.channels[1].lanes[0].add_clip(0, 4.0);
            clip.file.sample = Some(Box::default());
            let r = ClipRef {
                channel: 1,
                lane: 0,
                id: clip.id,
            };
            let mut s = Self {
                off: Offscreen::new(),
                editor: Editor {
                    editing: Some(r),
                    ..Default::default()
                },
                song,
                edits: Vec::new(),
                selected: Some(r),
            };
            s.frame(vec![]);
            s
        }
        fn frame(&mut self, events: Vec<egui::Event>) {
            let mut audio = AudioLink::default();
            let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
            let mut settings = Settings::default();
            let mut selected = 1;
            let r = self.selected.unwrap();
            let mut edit = None;
            let song = &mut self.song;
            let editor = &mut self.editor;
            self.off.frame(events, &mut |ui, cx| {
                let mut app = AppCtx {
                    cx: *cx,
                    regions: Regions::new(ui.max_rect(), cx.m),
                    song: &mut *song,
                    audio: &mut audio,
                    selected: &mut selected,
                    snapshot: None,
                    playing: true,
                    armed: false,
                    previewing: false,
                    middle_c_is_c4: false,
                    settings: &mut settings,
                    ghost_notes: true,
                    files: &mut files,
                    edit: None,
                    answer: None,
                    covered: false,
                    playhead: None,
                    seek: None,
                    midi_devices: &[],
                    midi_rescan: false,
                };
                let grid = ui.max_rect();
                editor.show(ui, &mut app, &[(r, BLOCK)], Some(r), grid, axis(), None);
                edit = app.edit;
            });
            self.edits.push(edit);
        }
        fn point(&self, lane: usize, tick: u32, v: f32) -> Pos2 {
            let m = crate::ui::offscreen::metrics();
            let r = value_rect(lanes(area(clip_box(BLOCK, &m), &m))[lane], &m);
            xy(f64::from(tick), v, 0.0, axis(), r)
        }
        fn button(&mut self, p: Pos2, down: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: down,
                    modifiers: Default::default(),
                },
            ]);
        }
        fn points(&self, lane: i16) -> Vec<Point> {
            points::points(
                &self.song.channels[1].lanes[0].clips[0],
                PointKey::Envelope(lane),
            )
        }
    }
    #[test]
    fn clip_envelope_display_only_meaningful_lanes_and_thousands_untouched() {
        let mut clip = empty_clip();
        clip.file.sample = Some(Box::default());
        assert!(!used(&clip, 0) && !used(&clip, 1));
        let default = base(&clip, 0);
        points::set_points(&mut clip, PointKey::Envelope(0), vec![Point { tick: 0, value: default, ..Default::default() }]);
        assert!(!used(&clip, 0));
        points::set_points(&mut clip, PointKey::Envelope(0), vec![Point { tick: 0, value: 12000, ..Default::default() }]);
        assert!(used(&clip, 0) && !used(&clip, 1));
        points::set_points(&mut clip, PointKey::Envelope(1), vec![Point { tick: 0, value: 0, ..Default::default() }]);
        assert!(used(&clip, 0) && used(&clip, 1));
        points::set_points(&mut clip, PointKey::Envelope(0), vec![]);
        assert!(!used(&clip, 0) && used(&clip, 1));
        let before = clip.clone();
        let mut off = Offscreen::new();
        off.frame(vec![], &mut |ui, cx| {
            let hidden = crate::ui::envelope_display::EnvelopeDisplay { visible: false, ..Default::default() };
            hidden.publish(ui.ctx());
            let n = ui.painter().add(egui::Shape::Noop);
            paint(ui.painter(), &clip, BLOCK, axis(), cx.m);
            let _ = n;
        });
        assert_eq!(clip, before);
        CURVES.with(|c| c.borrow_mut().clear());
        let untouched = empty_clip();
        off.frame(vec![], &mut |ui, cx| {
            crate::ui::envelope_display::EnvelopeDisplay::default().publish(ui.ctx());
            for _ in 0..10000 { paint(ui.painter(), &untouched, BLOCK, axis(), cx.m); }
        });
        CURVES.with(|c| assert!(c.borrow().is_empty()));
    }
    #[test]
    fn clip_envelope_playlist_create_drag_live_cancel_and_one_undo_transaction() {
        let mut r = Rig::new();
        let before = r.song.clone();
        let mut history = crate::history::History::new(&before);
        let from = r.point(0, 64, 0.8);
        r.button(from, true);
        assert_eq!(r.points(0).len(), 1);
        assert_eq!(
            r.edits.last().copied().flatten(),
            Some(crate::ui::views::SongEdit::Audition)
        );
        history.edited(&r.song, false);
        let to = r.point(0, 128, 0.2);
        r.frame(vec![egui::Event::PointerMoved(to)]);
        assert_eq!(r.points(0)[0].tick, 128);
        assert!((points::unit(r.points(0)[0].value) - 0.2).abs() < 0.001);
        history.edited(&r.song, false);
        r.button(to, false);
        history.edited(&r.song, true);
        assert_eq!(
            r.edits.last().copied().flatten(),
            Some(crate::ui::views::SongEdit::Final)
        );
        assert_eq!(
            r.song.channels[1].lanes[0].clips[0].visible_len_beats,
            before.channels[1].lanes[0].clips[0].visible_len_beats
        );
        assert_eq!(
            history.undo().unwrap().channels[1].lanes[0].clips[0].events,
            before.channels[1].lanes[0].clips[0].events
        );
        assert!(history.undo().is_none());
        assert!(r.points(1).is_empty());
        let initial = r.points(0);
        r.button(to, true);
        r.frame(vec![egui::Event::PointerMoved(from)]);
        r.frame(crate::ui::offscreen::key_tap(
            egui::Key::Escape,
            Default::default(),
        ));
        assert_eq!(r.points(0), initial);
    }
    #[test]
    fn clip_envelope_pan_lane_is_independent_center_is_exact_and_live_data_is_editor_data() {
        let mut r = Rig::new();
        let p = r.point(1, 64, 0.5);
        r.button(p, true);
        r.button(p, false);
        assert!(r.points(0).is_empty());
        assert!((i32::from(r.points(1)[0].value) - 32768).abs() <= 1);
        let pts = vec![
            Point {
                tick: 0,
                value: 0,
                shape: Shape {
                    kind: ShapeKind::Smooth,
                    tension: 80,
                },
            },
            Point {
                tick: 128,
                value: 65535,
                ..Default::default()
            },
        ];
        audio_editor::apply(
            &mut r.song,
            (1, 0, 1),
            &Action::Points {
                key: PointKey::Envelope(1),
                points: pts.clone(),
            },
        );
        assert_eq!(r.points(1), pts);
        let bytes = crate::flm::write(&r.song).unwrap();
        let loaded = crate::flm::read(&bytes).unwrap();
        assert_eq!(
            points::points(&loaded.channels[1].lanes[0].clips[0], PointKey::Envelope(1)),
            pts
        );
    }
    #[test]
    fn clip_envelope_display_values_match_prepared_audio_steps_and_cache() {
        for kind in [
            ShapeKind::Linear,
            ShapeKind::Square,
            ShapeKind::Smooth,
            ShapeKind::Stairs,
        ] {
            let pts = vec![
                Point {
                    tick: 0,
                    value: 65535,
                    shape: Shape { kind, tension: 30 },
                },
                Point {
                    tick: 128,
                    value: 0,
                    ..Default::default()
                },
            ];
            let steps = prepared(&pts);
            assert!(std::sync::Arc::ptr_eq(&steps, &prepared(&pts)));
            let mut clip = empty_clip();
            points::set_points(&mut clip, PointKey::Envelope(1), pts);
            for tick in 0..160 {
                let expected = clip
                    .events
                    .iter()
                    .filter_map(|e| match e {
                        Event::Envelope {
                            pos,
                            env_param: 1,
                            value,
                            ..
                        } if *pos <= tick => Some(*value),
                        _ => None,
                    })
                    .last()
                    .unwrap();
                assert!((value_at(&steps, 32768, f64::from(tick)) - expected).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn clip_envelope_loop_offset_mapping_is_same_for_each_repeat() {
        let mut clip = empty_clip();
        clip.loop_len_beats = 4.0;
        clip.offset_beats = 1.0;
        clip.start_tick = 256;
        clip.visible_len_beats = 8.0;
        let lp = clip.loop_ticks().unwrap();
        let start = f64::from(clip.start_tick) - clip.offset_beats * f64::from(PPQ);
        for repeat in 0..2 {
            let tick = start + repeat as f64 * lp + 128.0;
            let origin = start + ((tick - start) / lp).floor() * lp;
            assert_eq!(tick - origin, 128.0);
        }
    }
    #[test]
    #[ignore = "controlled release rendering workload"]
    fn clip_envelope_render_benchmark() {
        use banaforge_audio::sample::SampleData;
        use std::{hint::black_box, sync::Arc, time::Instant};
        let source = Waveform::new(Arc::new(SampleData {
            sample_rate: 48000,
            left: vec![0.25; 96000].into(),
            ..Default::default()
        }));
        let ctx = egui::Context::default();
        let m = crate::ui::offscreen::metrics();
        let wave = Wave {
            frames: 96000,
            per_beat: 24000.0,
            reversed: false,
        };
        let mesh = super::super::wave_mesh(
            &source,
            wave,
            &[(60.0, 20.0)],
            ((0.0, 4.0 / 640.0), (20.0, 640)),
            1.0,
            Color32::GRAY,
        );
        let mut clip = empty_clip();
        clip.loop_len_beats = 4.0;
        clip.file.sample = Some(Box::default());
        for points_count in [2, 32, 128] {
            for k in 0..2 {
                let pts = (0..points_count)
                    .map(|i| Point {
                        tick: i as u32 * 511 / (points_count - 1) as u32,
                        value: if i % 2 == 0 { 8000 } else { 58000 },
                        shape: Shape {
                            kind: if i % 3 == 0 {
                                ShapeKind::Square
                            } else {
                                ShapeKind::Smooth
                            },
                            tension: 50,
                        },
                    })
                    .collect();
                points::set_points(&mut clip, PointKey::Envelope(k), pts);
            }
            for repeat in 1..=3 {
                for enabled in [false, true] {
                    // Warm fonts, curve caches and egui before timing; source mesh is an unchanged cached Arc.
                    let frame = || {
                        ctx.run(
                            egui::RawInput {
                                screen_rect: Some(Rect::from_min_size(
                                    Pos2::ZERO,
                                    vec2(700.0, 1500.0),
                                )),
                                ..Default::default()
                            },
                            |ctx| {
                                let painter = ctx.layer_painter(egui::LayerId::background());
                                for i in 0..8 {
                                    let shift = vec2(0.0, i as f32 * 170.0);
                                    let mut mesh = mesh.as_ref().clone();
                                    mesh.translate(shift);
                                    painter.add(mesh);
                                    if enabled {
                                        super::paint(
                                            &painter,
                                            &clip,
                                            BLOCK.translate(shift),
                                            axis(),
                                            &m,
                                        );
                                    }
                                }
                            },
                        )
                    };
                    for _ in 0..3 {
                        black_box(frame());
                    }
                    let start = Instant::now();
                    let mut output = frame();
                    for _ in 0..63 {
                        output = frame();
                        black_box(&output);
                    }
                    let paint = start.elapsed().as_secs_f64() * 1000.0 / 64.0;
                    let start = Instant::now();
                    let mut primitives = ctx.tessellate(output.shapes.clone(), 1.0);
                    for _ in 0..63 {
                        primitives = ctx.tessellate(output.shapes.clone(), 1.0);
                        black_box(&primitives);
                    }
                    let tess = start.elapsed().as_secs_f64() * 1000.0 / 64.0;
                    let vertices: usize = primitives
                        .iter()
                        .map(|p| match &p.primitive {
                            egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                            _ => 0,
                        })
                        .sum();
                    let indices: usize = primitives
                        .iter()
                        .map(|p| match &p.primitive {
                            egui::epaint::Primitive::Mesh(m) => m.indices.len(),
                            _ => 0,
                        })
                        .sum();
                    println!(
                        "CLIP_ENVELOPE_BENCH points={points_count} enabled={enabled} repeat={repeat} clips=8 paint_ms={paint:.6} tess_ms={tess:.6} vertices={vertices} indices={indices}"
                    );
                }
            }
        }
    }
}
