//! Drag and drop of audio files onto the playlist.
//!
//! - **External files** dragged over the window (egui's hovered files, native and web; audio or project files, or
//!   files the platform does not name): a full-screen curtain fades in, `background` at α 0.85·p under `Drop to
//!   import files` (FontSize 4, centred, shrunk to fit) in `text` at α p. p eases toward 1 while files hover over the
//!   window and back to 0 once they leave or drop, frame-time based: p ← p + (target − p)·(1 − e^(−Δt/0.05)). It is
//!   drawn over everything.
//! - **In-app:** a long press (300 ms) then a drag (more than 0.5 U) on an audio file row of a side browser
//!   ([`row_dragged`]) drags that file ([`start`]); its name follows the pointer, just above it. Released over the
//!   playlist it drops there; anywhere else the drag is cancelled.
//! - **Drop target** ([`playlist`]): while a drag is over the playlist the playlist shows where it lands ([`place`]),
//!   in `accent` pulsing k = 0.9 + 0.1·sin(2π·0.4·t), with 2t lines: over an audio track a highlight across its row
//!   (α 0.05k at the row's ends rising to 0.25k at the drop x) and a vertical line at the snapped beat; otherwise an
//!   insertion line between channels (before or after the hovered channel, by the half of its rows the pointer is in),
//!   or after the last row (within half a row below it). It shows only where the pointer's position is known: egui
//!   reports none during an external drag on most platforms (X11, Wayland, the web), and then only the curtain shows.
//!   It is drawn over the curtain, which would hide it.
//!
//! [`end_frame`] draws all of it, last in the frame, and hands the app what was dropped ([`Dropped`]): the app makes the
//! clips (`app/file_drop.rs`).

use std::path::{Path, PathBuf};

use eframe::egui::{
    Align2, Color32, Context, DroppedFile, Id, InputState, LayerId, Mesh, Order, Pos2, Rect,
    Response, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::metrics::{FontSize, Metrics};
use crate::project_files::{is_audio, is_project};
use crate::song::{LaneKind, Song};
use crate::ui::clips::TimeAxis;
use crate::ui::{UiCtx, gesture, widgets};

/// The curtain's text.
pub const DROP_TO_IMPORT: &str = "Drop to import files";
/// The curtain's `background` alpha at p = 1.
const CURTAIN_ALPHA: f32 = 0.85;
/// The curtain's time constant (s).
const TAU_S: f32 = 0.05;
/// The highlight's alpha over an audio track, at its ends and at the drop x (times k).
const HIGHLIGHT_ALPHA: [f32; 2] = [0.05, 0.25];
/// The pulse's frequency (Hz) and depth: k = 1 − DEPTH + DEPTH·sin(2π·f·t).
const PULSE_HZ: f64 = 0.4;
const PULSE_DEPTH: f32 = 0.1;
/// The dragged name's box: its height and side padding, and its gap above the pointer, in U.
const LABEL_H: f32 = 1.5;
const LABEL_PAD: f32 = 0.25;
const LABEL_GAP: f32 = 0.25;

/// Where a drop on the playlist lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// On channel `channel`'s audio track `lane`, its clip at song tick `tick`.
    Track {
        channel: usize,
        lane: usize,
        tick: u32,
    },
    /// A new Audio channel inserted at channel index `at`, its clip at `tick`.
    Insert { at: usize, tick: u32 },
}

/// What was dropped this frame ([`end_frame`]).
pub enum Dropped {
    /// Audio files from outside the app, where they landed (`None`: off the playlist, or where egui gives no position).
    External(Vec<DroppedFile>, Option<Place>),
    /// A browser's audio file, dropped on the playlist.
    Library(PathBuf, Place),
}

/// What lives between frames, in the context's memory.
#[derive(Clone, Default)]
struct State {
    /// The in-app drag: the file and its row's name.
    dragged: Option<(PathBuf, String)>,
    /// The drop target the playlist found, with the frame it found it in and its shapes.
    target: Option<(u64, Place, Vec<Shape>)>,
    /// The curtain's p.
    p: f32,
}

fn id() -> Id {
    Id::new("file_drag")
}

fn state(ctx: &Context) -> State {
    ctx.data(|d| d.get_temp(id())).unwrap_or_default()
}

fn keep(ctx: &Context, state: State) {
    ctx.data_mut(|d| d.insert_temp(id(), state));
}

/// Starts an in-app drag of the audio file `path`, shown as `name` (a browser row's [`row_dragged`]). One under way
/// stays.
pub fn start(ctx: &Context, path: &Path, name: &str) {
    let mut s = state(ctx);
    if s.dragged.is_none() {
        s.dragged = Some((path.to_owned(), name.to_owned()));
        keep(ctx, s);
        ctx.request_repaint();
    }
}

/// An in-app drag is under way: the browser's list does not scroll with it.
pub fn dragging(ctx: &Context) -> bool {
    state(ctx).dragged.is_some()
}

/// Whether the press on the browser row `response` starts a drag of it this frame: held past its long press
/// ([`gesture::press`]), then moved out of the calm zone. The row's click stays egui's: a long press released where it
/// went down still picks. Call it every frame the row shows.
pub fn row_dragged(ui: &Ui, response: &Response, m: &Metrics) -> bool {
    let press = gesture::press(ui, &mut response.clone(), m, Vec2::length);
    press.held
        && ui.input(|i| {
            (i.pointer.press_origin())
                .is_some_and(|o| gesture::left_calm(o, i.pointer.latest_pos(), m.u(1.0)))
        })
}

/// External files hover over the window that `fits`, or that the platform does not name (the web).
fn hovering(i: &InputState, fits: fn(&Path) -> bool) -> bool {
    (i.raw.hovered_files.iter()).any(|f| f.path.as_deref().is_none_or(fits))
}

/// A file a drop does something with: an audio file, or a project file, which opens.
fn known(path: &Path) -> bool {
    is_audio(path) || is_project(path)
}

/// The audio files dropped on the window this frame; none when a project file is among them, which opens instead.
fn audio_dropped(i: &InputState) -> Vec<DroppedFile> {
    let path = |f: &DroppedFile| f.path.clone().unwrap_or_else(|| PathBuf::from(&f.name));
    if (i.raw.dropped_files.iter()).any(|f| is_project(&path(f))) {
        return Vec::new();
    }
    (i.raw.dropped_files.iter())
        .filter(|f| is_audio(&path(f)))
        .cloned()
        .collect()
}

/// The playlist's frame: while a drag is over `area` (the rows and their header column), finds where it lands
/// ([`place`]) and keeps it with the target's shapes for [`end_frame`]. `rows` are the playlist's rows (channel,
/// lane, top), `row_h` tall; `grid` is the clip grid on `axis`; `unit` the snap grid in ticks.
pub fn playlist(
    ui: &Ui,
    cx: &UiCtx,
    song: &Song,
    (area, grid): (Rect, Rect),
    (rows, row_h): (&[(usize, usize, f32)], f32),
    axis: TimeAxis,
    unit: f64,
) {
    let ctx = ui.ctx();
    let mut s = state(ctx);
    let pos = ui.input(|i| match &s.dragged {
        Some(_) => i.pointer.latest_pos(),
        None if hovering(i, is_audio) || !audio_dropped(i).is_empty() => i.pointer.hover_pos(),
        None => None,
    });
    let Some(pos) = pos.filter(|p| area.contains(*p)) else {
        return;
    };
    let Some((place, y)) = self::place(song, (rows, row_h), pos, grid.left(), axis, unit) else {
        return;
    };
    let (m, accent) = (cx.m, cx.theme.colors.accent.to_color32());
    let t = ui.input(|i| i.time);
    let k = 1.0 - PULSE_DEPTH + PULSE_DEPTH * (std::f64::consts::TAU * PULSE_HZ * t).sin() as f32;
    let line = Stroke::new(2.0 * m.t(), accent.gamma_multiply(k));
    let shapes = match place {
        Place::Track { tick, .. } => {
            let row = Rect::from_x_y_ranges(grid.x_range(), y..=y + row_h).intersect(area);
            let x = axis.x(f64::from(tick)).clamp(row.left(), row.right());
            let [edge, peak] = HIGHLIGHT_ALPHA.map(|a| accent.gamma_multiply(a * k));
            vec![
                highlight(row, pos.x.clamp(row.left(), row.right()), edge, peak),
                Shape::line_segment([pos2(x, row.top()), pos2(x, row.bottom())], line),
            ]
        }
        Place::Insert { .. } => {
            let y = y.clamp(area.top(), area.bottom());
            vec![Shape::line_segment(
                [pos2(area.left(), y), pos2(area.right(), y)],
                line,
            )]
        }
    };
    s.target = Some((ctx.cumulative_frame_nr(), place, shapes));
    keep(ctx, s);
    // The pulse runs.
    ctx.request_repaint();
}

/// `row` filled `edge` at its ends, rising linearly to `peak` at `x`.
fn highlight(row: Rect, x: f32, edge: Color32, peak: Color32) -> Shape {
    let mut mesh = Mesh::default();
    for (x, color) in [(row.left(), edge), (x, peak), (row.right(), edge)] {
        mesh.colored_vertex(pos2(x, row.top()), color);
        mesh.colored_vertex(pos2(x, row.bottom()), color);
    }
    for i in [0, 2] {
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 2, i + 3);
    }
    Shape::mesh(mesh)
}

/// Where a file dropped at `pos` lands over the playlist rows `rows` (channel, lane, top), each `row_h` tall, with the
/// y of what marks it (the track's top, or the insertion line):
/// - over an audio track, that track ([`Place::Track`]);
/// - over another track, a new channel ([`Place::Insert`]) before its channel when `pos` is in the upper half of the
///   channel's rows, after it in the lower half; within half a row below the last row, after the last channel;
/// - `None` anywhere else.
///
/// The tick is `pos.x` on `axis` rounded to the grid `unit`, and 0 left of the grid (`grid_left`: the header column).
/// The new channel's index is that of the first channel below the line, the master never: the master, shown where
/// [`Song::master_below`] puts it, stays below the channel it follows.
pub fn place(
    song: &Song,
    (rows, row_h): (&[(usize, usize, f32)], f32),
    pos: Pos2,
    grid_left: f32,
    axis: TimeAxis,
    unit: f64,
) -> Option<(Place, f32)> {
    let tick = match pos.x < grid_left {
        true => 0,
        false => ((axis.tick(pos.x).max(0.0) / unit).round() * unit).round() as u32,
    };
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut order: Vec<usize> = rows.iter().map(|r| r.0).collect();
    order.dedup();
    // A new channel at playlist place `p` goes in at the index of the first channel from there on.
    let at = |p: usize| {
        (order.get(p..).unwrap_or_default().iter())
            .copied()
            .find(|&c| c != 0)
            .unwrap_or(song.channels.len())
    };
    let Some(&(channel, lane, top)) = rows.iter().find(|r| (r.2..r.2 + row_h).contains(&pos.y))
    else {
        let bottom = rows.last()?.2 + row_h;
        let below = (bottom..bottom + row_h / 2.0).contains(&pos.y);
        return below.then(|| {
            (
                Place::Insert {
                    at: at(order.len()),
                    tick,
                },
                bottom,
            )
        });
    };
    let lane_kind = song.channels.get(channel).and_then(|c| c.lanes.get(lane));
    if lane_kind.is_some_and(|l| l.kind == LaneKind::Audio) {
        return Some((
            Place::Track {
                channel,
                lane,
                tick,
            },
            top,
        ));
    }
    let tops = rows.iter().filter(|r| r.0 == channel).map(|r| r.2);
    let (first, last) = tops.fold((top, top), |(a, b), t| (a.min(t), b.max(t)));
    let p = order.iter().position(|&c| c == channel)?;
    Some(match pos.y < (first + last + row_h) / 2.0 {
        true => (Place::Insert { at: at(p), tick }, first),
        false => (
            Place::Insert {
                at: at(p + 1),
                tick,
            },
            last + row_h,
        ),
    })
}

/// The frame's end: draws the curtain, the drop target the playlist found this frame and the dragged name, over
/// everything, and returns what was dropped this frame. An in-app drag ends with its press: dropped where the playlist
/// found a target, cancelled anywhere else.
pub fn end_frame(ctx: &Context, cx: &UiCtx) -> Option<Dropped> {
    let mut s = state(ctx);
    let frame = ctx.cumulative_frame_nr();
    let place = (s.target.take()).filter(|t| t.0 == frame);
    let (hover, dt, files, down, pointer) = ctx.input(|i| {
        let p = &i.pointer;
        (
            hovering(i, known),
            i.stable_dt,
            audio_dropped(i),
            p.primary_down(),
            p.latest_pos(),
        )
    });
    let goal = if hover { 1.0 } else { 0.0 };
    s.p += (goal - s.p) * (1.0 - (-dt / TAU_S).exp());
    if (goal - s.p).abs() < 1e-3 {
        s.p = goal;
    } else {
        ctx.request_repaint();
    }
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, id()));
    let (c, m) = (&cx.theme.colors, cx.m);
    let screen = ctx.content_rect();
    if s.p > 0.0 {
        let fill = c
            .background
            .to_color32()
            .gamma_multiply(CURTAIN_ALPHA * s.p);
        painter.rect_filled(screen.expand(1.0), 0.0, fill);
        let ink = c.text.to_color32().gamma_multiply(s.p);
        let pt = FontSize::Pt18.points() as u8;
        let line = screen.shrink2(vec2(m.u(LABEL_PAD), 0.0));
        widgets::shrink(
            &painter,
            m,
            line,
            Align2::CENTER_CENTER,
            DROP_TO_IMPORT,
            pt,
            ink,
        );
    }
    if let Some((_, _, shapes)) = &place {
        painter.extend(shapes.iter().cloned());
    }
    if let (Some((_, name)), Some(at)) = (&s.dragged, pointer) {
        let font = m.font_id(FontSize::Pt18);
        let ink = c.text.to_color32();
        let galley = painter.layout_no_wrap(name.clone(), font, ink);
        let size = vec2(galley.size().x + 2.0 * m.u(LABEL_PAD), m.u(LABEL_H));
        let rect = Rect::from_center_size(at - vec2(0.0, m.u(LABEL_GAP) + size.y / 2.0), size);
        let fill = c.menu_fill.to_color32();
        painter.rect(
            rect,
            m.u(0.15),
            fill,
            Stroke::new(m.t(), ink),
            StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
    }
    let place = place.map(|t| t.1);
    let drop = if !files.is_empty() {
        Some(Dropped::External(files, place))
    } else if !down && s.dragged.is_some() {
        let (path, _) = s.dragged.take().expect("dragged");
        ctx.request_repaint();
        place.map(|place| Dropped::Library(path, place))
    } else {
        None
    };
    keep(ctx, s);
    drop
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::ChannelKind;

    #[test]
    fn a_drop_lands_on_an_audio_track_at_the_snapped_beat_or_between_channels_by_the_half() {
        // Master, an Instrument with a second (notes) track, an Audio channel: rows 2 U = 64 points tall from y 0, the
        // grid from x 100 at 32 points a beat.
        let mut song = Song::default();
        song.add_channel(ChannelKind::Instrument);
        song.add_channel(ChannelKind::AudioClip);
        song.channels[1].add_lane(LaneKind::Notes);
        let rows = [(0, 0, 0.0), (1, 0, 64.0), (1, 1, 128.0), (2, 0, 192.0)];
        let axis = TimeAxis {
            origin: 100.0,
            beat_w: 32.0,
        };
        let beat = f64::from(crate::song::PPQ);
        let at = |x, y| place(&song, (&rows, 64.0), pos2(x, y), 100.0, axis, beat);
        // The audio track: beat 2.4 snaps to beat 2; left of the grid, beat 0.
        let track = |tick| Place::Track {
            channel: 2,
            lane: 0,
            tick,
        };
        assert_eq!(at(100.0 + 2.4 * 32.0, 200.0), Some((track(256), 192.0)));
        assert_eq!(at(50.0, 255.0), Some((track(0), 192.0)));
        // The Instrument's two rows: the upper half (its first row) goes before it, the lower after it.
        let insert = |at, tick| Place::Insert { at, tick };
        assert_eq!(at(132.0, 100.0), Some((insert(1, 128), 64.0)));
        assert_eq!(at(132.0, 150.0), Some((insert(2, 128), 192.0)));
        // The master, at the top: before or after it, the new channel goes below it.
        assert_eq!(at(132.0, 10.0), Some((insert(1, 128), 0.0)));
        // Within half a row below the last row: after the last channel; further down, nowhere.
        assert_eq!(at(132.0, 256.0 + 31.0), Some((insert(3, 128), 256.0)));
        assert_eq!(at(132.0, 256.0 + 33.0), None);
    }
}
