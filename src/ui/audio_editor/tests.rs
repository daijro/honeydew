use std::path::Path;
use std::sync::Arc;

use eframe::egui::epaint::ClippedShape;
use eframe::egui::{Color32, Pos2, Rect, Shape, UiBuilder, Vec2, pos2, vec2};
use honeydew_audio::sample::{SampleData, SampleMeta};
use honeydew_audio::sample_edit::peaks;

use super::*;
use crate::audio_link::AudioLink;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::metrics::Metrics;
use crate::project_files::ProjectFiles;
use crate::project_files::mem::MemFiles;
use crate::settings::Settings;
use crate::song::{ChannelKind, TimeSig};
use crate::ui::automation;
use crate::ui::drum_grid::edit::marks;
use crate::ui::main_screen::Regions;
use crate::ui::offscreen::{Offscreen, SCREEN, metrics};
use crate::ui::piano_roll::notes::auto_grid;
use crate::ui::views::{self, AppCtx, SongEdit};

/// The editor's rect in the rig: its ruler from a little above the screen's top down to y 48 (so that all below it
/// lies on whole points), grid x 50..1170 and y 48..340, value area y 64..324 (V = 260).
const RECT: Rect = Rect {
    min: pos2(0.0, 48.0 - 52.0),
    max: pos2(1170.0, 340.0),
};
const AT: At = (1, 0, 1);
const VOL: PointKey = PointKey::Envelope(0);

fn frame() -> Frame {
    Frame::new(RECT, &metrics())
}

/// An Audio Clip channel (1) with one clip at the song start, 4 beats, at 120 BPM in 4/4.
fn song() -> Song {
    let mut song = Song {
        tempo_bpm: 120.0,
        ..Song::default()
    };
    song.add_channel(ChannelKind::AudioClip);
    song.channels[1].lanes[0].add_clip(0, 4.0);
    song.channels[1].lanes[0].clips[0].file.sample = Some(Box::default());
    song
}

fn slot(song: &mut Song) -> &mut SamplerSlot {
    song.channels[1].lanes[0].clips[0]
        .file
        .sample
        .as_deref_mut()
        .unwrap()
}

fn clip(song: &Song) -> &Clip {
    &song.channels[1].lanes[0].clips[0]
}

fn sample(rate: u32, left: Vec<f32>, right: Option<Vec<f32>>) -> SampleData {
    SampleData {
        sample_rate: rate,
        left: left.into(),
        right: right.map(Into::into),
        meta: SampleMeta::default(),
    }
}

/// The snap grid at the initial 4 U a beat (Snap on, Auto).
fn snap() -> Option<f64> {
    Snap::default().active(
        auto_grid(128.0, 32.0, TimeSig::default()),
        TimeSig::default(),
    )
}

struct Rig {
    off: Offscreen,
    song: Song,
    ed: AudioEditor,
    sample: Option<Waveform>,
    t: Transport,
    /// Each frame's actions.
    log: Vec<Vec<Action>>,
    rect: Rect,
    shapes: Vec<ClippedShape>,
}

impl Rig {
    fn new(song: Song, sample: Option<SampleData>) -> Self {
        let mut rig = Self {
            off: Offscreen::new(),
            song,
            ed: AudioEditor::default(),
            sample: sample.map(|s| Waveform::new(Arc::new(s))),
            t: Transport::default(),
            log: Vec::new(),
            shapes: Vec::new(),
            rect: RECT,
        };
        rig.run(vec![]);
        rig
    }

    /// One frame; the edits apply to the song as the view applies them.
    fn run(&mut self, events: Vec<egui::Event>) {
        let Self {
            off,
            song,
            ed,
            sample,
            t,
            log,
            rect,
            ..
        } = self;
        let mut actions = Vec::new();
        self.shapes = off.frame(events, &mut |ui, cx| {
            let mut ui = ui.new_child(UiBuilder::new().max_rect(*rect));
            actions = ed.show(&mut ui, cx, song, AT, sample.as_ref(), *t);
        });
        for a in &actions {
            if let Action::PlayLoop(l) = a {
                song.play_loop = *l;
            } else {
                apply(song, AT, a);
            }
        }
        log.push(actions);
    }

    fn click(&mut self, p: Pos2) {
        self.run(vec![egui::Event::PointerMoved(p)]);
        self.button(p, true);
        self.button(p, false);
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.run(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.run(vec![egui::Event::PointerMoved(from)]);
        self.button(from, true);
        for k in 1..=4 {
            self.run(vec![egui::Event::PointerMoved(
                from + (to - from) * k as f32 / 4.0,
            )]);
        }
        self.button(to, false);
    }

    /// Every action since the last call.
    fn take(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.log)
            .into_iter()
            .flatten()
            .collect()
    }

    fn rects(&self, fill: Color32) -> Vec<Rect> {
        (self.shapes.iter())
            .filter_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == fill => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// The waveform's columns in `fill`: its mesh's quads ([`add_column`]).
    fn quads(&self, fill: Color32) -> Vec<Rect> {
        let quads = mesh_quads(self.shapes.iter().map(|s| &s.shape)).into_iter();
        quads.filter(|q| q.1 == fill).map(|q| q.0).collect()
    }
}

fn wave_color(song: &Song) -> Color32 {
    let theme = crate::theme::theme_by_id(0);
    let ink = channel_color(song.channels[1].hue, &theme);
    mix(theme.colors.text, ink, 1.0 - WAVE_TEXT)
        .to_color32()
        .gamma_multiply(WAVE_ALPHA)
}

/// As the automation editor's: at beat 0 the line's left half shows in the t gap after the value column, which covers
/// only what lies left of its right edge.
#[test]
fn at_beat_0_the_lines_left_half_shows_in_the_gap_after_the_value_column() {
    let text = crate::theme::theme_by_id(0).colors.text.to_color32();
    let mut rig = Rig::new(song(), None);
    rig.t = Transport::default();
    rig.run(vec![]);
    let f = frame();
    // Down the grid (the lane list's selection bar is `text` too, in the column).
    let line = (rig.shapes.iter()).find_map(|s| match &s.shape {
        Shape::Rect(r) if r.fill == text && r.rect.y_range() == f.grid.y_range() => {
            Some((r.rect, s.clip_rect))
        }
        _ => None,
    });
    let (rect, clip) = line.expect("the line at tick 0");
    assert_eq!(
        crate::ui::timeline::testing::line_x(rect, 1.0),
        f.grid.left()
    );
    // Clipped at the column's right edge, so its left half shows left of the grid.
    assert_eq!(clip.left(), f.column.right());
    let shown = rect.intersect(clip);
    assert!(shown.left() < f.grid.left(), "{shown:?}");
    assert_eq!(shown.right(), rect.right());
}

/// The shared playhead line (the audio editor uses the automation grid): `text` down the grid at the playhead, stopped
/// as well as playing.
#[test]
fn the_playhead_line_shows_stopped_and_playing() {
    let text = crate::theme::theme_by_id(0).colors.text.to_color32();
    let mut rig = Rig::new(song(), None);
    for playing in [false, true] {
        rig.t = Transport {
            tick: 128,
            playing,
            ..Transport::default()
        };
        rig.run(vec![]);
        let grid = frame().grid;
        let lines: Vec<Rect> = (rig.rects(text).into_iter())
            .filter(|r| r.width() < 32.0 && r.y_range() == grid.y_range())
            .collect();
        assert_eq!(lines.len(), 1, "playing {playing}");
    }
}

/// The play box fades as the drum editor's (`timeline::knob_shown`, through `timeline::PlayheadKnob`): out while
/// playing, so that only the line shows, and back in once stopped.
#[test]
fn the_play_box_fades_out_while_playing_and_back_in_stopped() {
    let menu = crate::theme::theme_by_id(0).colors.menu_fill.to_color32();
    let mut rig = Rig::new(song(), None);
    let disc = |rig: &Rig| {
        crate::ui::timeline::testing::play_boxes(&rig.shapes)
            .next()
            .map(|b| b.fill)
    };
    for (playing, fill) in [(false, Some(menu)), (true, None), (false, Some(menu))] {
        rig.t = Transport {
            tick: 128,
            playing,
            ..Transport::default()
        };
        for _ in 0..40 {
            rig.run(vec![]);
        }
        assert_eq!(disc(&rig), fill, "playing {playing}");
    }
}

// ---- pure ----

#[test]
fn the_region_menu_has_the_audio_items_in_two_columns() {
    let range = TimeRange {
        start: 128,
        end: 256,
    };
    let items = region::audio_items(range, Some(0.0));
    let at: Vec<(&str, usize, usize)> = (items.iter())
        .map(|i| (i.label.as_ref(), i.col, i.row))
        .collect();
    assert_eq!(
        at,
        [
            ("Deselect", 0, 0),
            ("Insert silence", 0, 1),
            ("Duplicate", 0, 2),
            ("Copy to playhead", 0, 3),
            ("Move to playhead", 0, 4),
            ("Clear", 0, 5),
            ("Delete space", 0, 6),
            ("Trim selection", 0, 7),
            ("Normalize", 1, 0),
            ("Fade In", 1, 1),
            ("Fade Out", 1, 2),
        ]
    );
    assert!(items.iter().all(|i| !i.disabled));
    // The playhead items need the playhead outside the range.
    for playhead in [Some(130.0), None] {
        let disabled: Vec<String> = (region::audio_items(range, playhead).into_iter())
            .filter(|i| i.disabled)
            .map(|i| i.label.into_owned())
            .collect();
        assert_eq!(disabled, ["Copy to playhead", "Move to playhead"]);
    }
}

#[test]
fn the_beat_marks_are_never_closer_than_2_4_u() {
    // 4 U a beat: halves would be 2 U apart, so beats and bars only; at 1 U a beat, bars only.
    let ticks = |m: Vec<(f64, f32)>| {
        m.into_iter()
            .map(|(t, w)| (t as u32, w as u32))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ticks(marks(0.0, 512.0, 128.0, 32.0, TimeSig::default(), MARK_GAP)),
        [(0, 4), (128, 2), (256, 2), (384, 2), (512, 4)]
    );
    assert_eq!(
        ticks(marks(0.0, 1024.0, 32.0, 32.0, TimeSig::default(), MARK_GAP)),
        [(0, 4), (512, 4), (1024, 4)]
    );
    // 10 U a beat: quarters are 2.5 U apart.
    assert_eq!(
        ticks(marks(0.0, 64.0, 320.0, 32.0, TimeSig::default(), MARK_GAP)).len(),
        3
    );
}

#[test]
fn the_wave_scale_includes_speed_and_stretch() {
    // 60·rate/tempo frames a beat at speed 1: 1000 at 1 kHz and 60 BPM.
    let w = Wave::new(4000, 1000, 60.0, 0.5, 1.0, false);
    assert_eq!((w.per_beat, w.end()), (1000.0, 4.0));
    // Speed +12 semitones plays twice as fast: twice the frames a beat, half the length.
    let w = Wave::new(4000, 1000, 60.0, 0.5 + 12.0 / 48.0, 1.0, false);
    assert_eq!((w.per_beat, w.end()), (2000.0, 2.0));
    // A stretched (SYNC) slot is drawn at its stretched length.
    let w = Wave::new(4000, 1000, 60.0, 0.5, 1.5, false);
    assert_eq!(w.end(), 6.0);
    let mut s = SamplerSlot::default();
    assert_eq!(stretch_of(&s), 1.0);
    s.file.stretch_ratio = 1.5;
    assert_eq!(stretch_of(&s), 1.0, "only while stretch is on");
    s.file.stretch_mode = 1;
    assert_eq!(stretch_of(&s), 1.5);
    s.file.stretch_ratio = 0.0;
    assert_eq!(stretch_of(&s), 1.0, "no ratio from a file");
}

#[test]
fn waveform_columns_are_the_peaks_mirrored_for_a_reversed_sample() {
    let ramp: Vec<f32> = (0..8).map(|i| i as f32).collect();
    // 2 frames a column: 1 frame a beat, half a beat a column.
    let w = Wave::new(8, 1, 60.0, 0.5, 1.0, false);
    assert_eq!(
        columns(&ramp, &[], w, 0.0, 2.0, 4),
        [(0.0, 1.0), (2.0, 3.0), (4.0, 5.0), (6.0, 7.0)]
    );
    assert_eq!(
        columns(&ramp, &[], w, 1.0, 2.0, 2),
        peaks(&ramp, &[], 2.0, 1.0, 2)
    );
    let rev = Wave {
        reversed: true,
        ..w
    };
    assert_eq!(
        columns(&ramp, &[], rev, 0.0, 2.0, 4),
        [(6.0, 7.0), (4.0, 5.0), (2.0, 3.0), (0.0, 1.0)]
    );
    // Past the end: nothing (the editor clips to the sample's extent).
    assert_eq!(columns(&ramp, &[], rev, 2.0, 2.0, 4)[3], (0.0, 0.0));
    assert_eq!(columns(&ramp, &[], w, 2.0, 2.0, 4)[3], (0.0, 0.0));
}

#[test]
fn a_column_under_a_pixel_is_drawn_as_a_1_px_line() {
    for px in [1.0, 0.5] {
        // Full scale fills the scale both ways.
        assert_eq!(
            column_span(-1.0, 1.0, 100.0, 50.0, px),
            Rangef::new(50.0, 150.0)
        );
        // Silence: a 1 px line on the centre.
        let line = column_span(0.0, 0.0, 100.0, 50.0, px);
        assert_eq!(line, Rangef::new(100.0, 100.0 + px));
        // Just under a pixel.
        assert_eq!(
            column_span(0.0, 0.9 * px / 50.0, 100.0, 50.0, px).span(),
            px
        );
    }
}

#[test]
fn mono_is_centred_and_stereo_halves_at_a_quarter_and_three_quarters() {
    let area = Rect::from_min_size(pos2(0.0, 64.0), vec2(100.0, 260.0));
    assert_eq!(wave_rows(area, false), [(194.0, 130.0)]);
    assert_eq!(wave_rows(area, true), [(129.0, 65.0), (259.0, 65.0)]);
}

#[test]
fn geometry_at_ppp_1_and_2() {
    for ppp in [1.0, 2.0] {
        let m = Metrics::from_pixels_per_point(ppp, 100);
        let ruler = crate::ui::timeline::ruler_h(&m);
        let f = Frame::new(RECT.with_min_y(48.0 - ruler), &m);
        // One column a device pixel across the grid.
        assert_eq!(pixel_columns(f.grid, &m), (50.0, (1120.0 * ppp) as usize));
        // The sample-length handle: the end capsule from 0.15 U right of the end line, centred on the wave area; its
        // box 2 U, centred on it.
        let (h, px) = (stretch_handle(&f, 306.0, &m), 1.0 / ppp);
        assert!((h.left() - (306.0 + 4.8)).abs() < 1e-3 && h.center().y == f.value.center().y);
        assert!((h.width() - (13.44 + 2.0 * px)).abs() < 1e-3);
        let touch = stretch_box(&f, 306.0, &m);
        assert_eq!((touch.left(), touch.right()), (h.left(), 306.0 + 64.0));
        assert_eq!((touch.center().y, touch.height()), (h.center().y, 64.0));
    }
}

#[test]
fn the_stretch_length_snaps_in_song_beats_and_clamps_to_0_2_to_10_sources() {
    // The clip's content starts half a beat into the song; a beat grid.
    let at = |tick| stretch_len(tick, 64, Some(128.0), 1.0);
    // Content 300 = song 364 → 384 → 320 ticks.
    assert_eq!(at(300.0), 2.5);
    assert_eq!(at(10_000.0), 10.0);
    assert_eq!(at(-100.0), 0.2);
    // Snap off: whole ticks.
    assert_eq!(stretch_len(300.4, 64, None, 4.0), 300.0 / 128.0);
    // ratio = trunc(rate·60·L/tempo)/source frames.
    assert_eq!(stretch_ratio(2.0, 44_100, 120.0, 44_100), 1.0);
    assert_eq!(stretch_ratio(3.0, 44_100, 120.0, 44_100), 1.5);
    assert_eq!(stretch_ratio(1.0, 1000, 7.0, 1000), 8.571);
}

#[test]
fn a_region_ops_envelope_part_follows_automation_s_rows() {
    let mut s = song();
    s.play_loop = Some((128, 256));
    let pts = vec![
        Point {
            tick: 0,
            value: 0,
            ..Default::default()
        },
        Point {
            tick: 512,
            value: 65535,
            ..Default::default()
        },
    ];
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts.clone());
    let range = TimeRange {
        start: 128,
        end: 256,
    };
    let region = |op| Action::Region {
        key: VOL,
        op,
        range,
        playhead: 0,
    };
    // `Clear` is the envelope's `Delete`.
    let mut want = pts.clone();
    points::delete(&mut want, 128, 256);
    assert!(apply(&mut s, AT, &region(RegionOp::Clear)));
    assert_eq!(points::points(clip(&s), VOL), want);
    assert_eq!(s.play_loop, None, "every op but Duplicate clears the range");
    // Audio only: no envelope part.
    let before = s.clone();
    assert!(!apply(&mut s, AT, &region(RegionOp::Normalize)));
    assert_eq!(s, before);
    // Duplicate keeps the range; Insert silence is the envelope's Insert space.
    s.play_loop = Some((128, 256));
    apply(&mut s, AT, &region(RegionOp::Duplicate));
    assert_eq!(s.play_loop, Some((128, 256)));
    let mut want = points::points(clip(&s), VOL);
    points::insert_space(&mut want, 128, 256);
    apply(&mut s, AT, &region(RegionOp::InsertSilence));
    assert_eq!(points::points(clip(&s), VOL), want);
    // The loop follows the content.
    assert_eq!(clip(&s).loop_len_beats, 8.0);
}

#[test]
fn a_stretch_sets_the_ratio_and_the_clip_resyncs_by_the_loop_rule() {
    // A 4-beat clip: L = 3 makes @28 = 3 beats, and the loop stays 4 (never below 4 or b, @8).
    // A slot not stretched yet: the length turns SYNC on.
    let mut s = song();
    let stretch = |len_beats, ratio| Action::Stretch { len_beats, ratio };
    assert!(apply(&mut s, AT, &stretch(3.0, 1.5)));
    let c = clip(&s);
    let file = &c.file.sample.as_ref().unwrap().file;
    assert_eq!((file.stretch_ratio, file.stretch_mode), (1.5, 1));
    assert_eq!((c.file.source_len_beats, c.loop_len_beats), (3.0, 4.0));
    // The next envelope edit keeps that loop.
    let point = vec![Point {
        tick: 0,
        value: 1,
        ..Default::default()
    }];
    apply(
        &mut s,
        AT,
        &Action::Points {
            key: VOL,
            points: point,
        },
    );
    assert_eq!(clip(&s).loop_len_beats, 4.0);
    // The same again changes nothing.
    assert!(!apply(&mut s, AT, &stretch(3.0, 1.5)));
    // @28 includes Speed: at +12 semitones L = 6 plays in 3 beats; the loop holds trunc(4·3)/4 → 4.
    slot(&mut s).params[2] = 0.75;
    apply(&mut s, AT, &stretch(6.0, 3.0));
    assert_eq!(
        (clip(&s).file.source_len_beats, clip(&s).loop_len_beats),
        (3.0, 4.0)
    );
    // Longer: the loop grows to 4·2ᵐ. A set visible length stays, one equal to the loop included.
    slot(&mut s).params[2] = 0.5;
    s.channels[1].lanes[0].clips[0].visible_len_beats = 4.0;
    apply(&mut s, AT, &stretch(6.0, 3.0));
    assert_eq!(
        (clip(&s).loop_len_beats, clip(&s).visible_len_beats),
        (8.0, 4.0)
    );
    s.channels[1].lanes[0].clips[0].visible_len_beats = 5.0;
    apply(&mut s, AT, &stretch(10.0, 5.0));
    assert_eq!(
        (clip(&s).loop_len_beats, clip(&s).visible_len_beats),
        (16.0, 5.0)
    );
    // A visible length of 0 shows the source (@0 → @28), the loop 4·2ᵐ around it.
    s.channels[1].lanes[0].clips[0].visible_len_beats = 0.0;
    apply(&mut s, AT, &stretch(6.0, 3.0));
    let c = clip(&s);
    assert_eq!((c.loop_len_beats, c.visible_len_beats), (8.0, 0.0));
    assert_eq!(c.visible_ticks(), 6.0 * 128.0);
}

#[test]
fn the_zoom_limits_use_the_subdivision_d() {
    // (12/U)·4/d … (0.01/U)·4/d beats per point, d = TDIV byte 0 (the shared frame's).
    let sig = |d, b| TimeSig {
        subdivision: d,
        beats_per_bar: b,
    };
    assert_eq!(geom::zoom_limits(32.0, sig(3, 4)), [2.0, 2400.0]);
    assert_eq!(
        geom::zoom_limits(32.0, sig(3, 7)),
        geom::zoom_limits(32.0, sig(3, 4))
    );
    // The view zoomed all the way out at d = 3 (b = 7 plays no part).
    let mut s = song();
    s.time_sig = sig(3, 7);
    let mut rig = Rig::new(s, None);
    rig.run(vec![egui::Event::PointerMoved(pos2(600.0, 200.0))]);
    for _ in 0..10 {
        rig.run(vec![egui::Event::Zoom(0.01)]);
    }
    assert_eq!(rig.ed.views[&AT].beat_w, 2.0);
    for _ in 0..10 {
        rig.run(vec![egui::Event::Zoom(100.0)]);
    }
    assert_eq!(rig.ed.views[&AT].beat_w, 2400.0);
}

// ---- the view, offscreen ----

#[test]
fn a_tap_adds_a_vol_point_and_a_drag_moves_it_in_one_final_edit() {
    let mut rig = Rig::new(song(), None);
    let f = frame();
    rig.click(pos2(178.0, 194.0));
    let (pts, _) = points::create(&[], 128.0, f.row_at(194.0), snap(), 65535);
    assert_eq!(
        rig.take(),
        [Action::Points {
            key: VOL,
            points: pts.clone()
        }]
    );
    assert_eq!(points::points(clip(&rig.song), VOL), pts);
    // The new point is selected; a drag on it moves it, one edit at the release.
    let p = pos2(178.0, f.point_y(pts[0].value, 65535));
    rig.drag(p, p + vec2(128.0, -26.0));
    let dr = f64::from(-26.0 * 127.0 / f.value.height());
    let (moved, _) = points::moved(&pts, &[true], 128.0, dr, snap(), 65535).unwrap();
    assert_eq!(
        rig.take(),
        [Action::Points {
            key: VOL,
            points: moved.clone()
        }]
    );
    assert_eq!(points::points(clip(&rig.song), VOL), moved);
    // The moved point stays selected but quiet: a tap on it shows it again.
    rig.click(p + vec2(128.0, -26.0));
    // A tap on empty grid with the point showing only deselects.
    rig.click(pos2(700.0, 300.0));
    assert_eq!(rig.take(), []);
    assert_eq!(rig.ed.sel, [false]);
}

#[test]
fn the_lane_list_picks_the_envelope_the_grid_edits() {
    let mut rig = Rig::new(song(), None);
    let theme = crate::theme::theme_by_id(0);
    let ink = channel_color(rig.song.channels[1].hue, &theme).to_color32();
    let text = theme.colors.text.to_color32();
    let labels = Offscreen::texts(&rig.shapes);
    assert!(labels.contains(&("VOL".into(), text)), "{labels:?}");
    assert!(labels.contains(&("PAN".into(), ink)));
    assert!(labels.contains(&("SPEED".into(), ink)));
    // Equal rows down the 1.5 U column below the ruler: PAN in the middle third.
    rig.click(pos2(24.0, 194.0));
    assert_eq!(rig.ed.lane, 1);
    rig.click(pos2(306.0, 100.0));
    match &rig.take()[..] {
        [Action::Points { key, points }] => {
            assert_eq!((*key, points.len()), (PointKey::Envelope(1), 1));
        }
        other => panic!("{other:?}"),
    }
    // The lane's own points only: VOL is still empty.
    assert!(points::points(clip(&rig.song), VOL).is_empty());
}

#[test]
fn a_long_press_marquees_and_copy_then_paste_adds_the_points() {
    let mut s = song();
    let pts = vec![
        Point {
            tick: 128,
            value: 40000,
            ..Default::default()
        },
        Point {
            tick: 256,
            value: 20000,
            ..Default::default()
        },
    ];
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts.clone());
    let mut rig = Rig::new(s, None);
    // A long press on empty grid, then a move: the box.
    let from = pos2(120.0, 70.0);
    rig.run(vec![egui::Event::PointerMoved(from)]);
    rig.button(from, true);
    for _ in 0..20 {
        rig.run(vec![]);
    }
    assert!(rig.ed.paste.is_none(), "an empty clipboard offers no Paste");
    let to = pos2(400.0, 320.0);
    rig.run(vec![egui::Event::PointerMoved(to)]);
    rig.button(to, false);
    assert_eq!(rig.ed.sel, [true, true]);
    assert_eq!(rig.take(), []);
    // The marquee's release shows the floating row: its `Copy`.
    let copy = rig
        .off
        .ctx
        .read_response(egui::Id::new(("audio_point_button", 0usize)));
    rig.click(copy.expect("the floating row").rect.center());
    assert_eq!(
        rig.ed.sel,
        [true, true],
        "the row's press is not the grid's"
    );
    let copied = automation::clipboard(&rig.off.ctx);
    assert_eq!(copied, points::copy(&pts, &[true, true], 65535));
    // A long press without moving shows `Paste`; its tap pastes at the button's beat.
    rig.click(pos2(700.0, 300.0));
    let at = pos2(572.0, 200.0);
    rig.run(vec![egui::Event::PointerMoved(at)]);
    rig.button(at, true);
    for _ in 0..20 {
        rig.run(vec![]);
    }
    assert!(rig.ed.paste.is_some(), "Paste appears before release");
    let menu = egui::Id::new("audio_editor").with("paste");
    let paste = crate::ui::insert::button(&rig.off.ctx, menu, 0).expect("the Paste menu");
    assert!(paste.bottom() < at.y, "over the finger");
    assert_eq!(rig.take(), []);
    rig.button(at, false);
    assert_eq!(rig.take(), [], "the original lift does not paste");
    rig.click(paste.center());
    // Tick 522 snaps to 512.
    let (want, _) = points::paste(&pts, &copied, 522.0, snap(), 65535);
    assert_eq!(want[2].tick, 512);
    assert_eq!(
        rig.take(),
        [Action::Points {
            key: VOL,
            points: want
        }]
    );
    // A held bubble disappears when the gesture becomes a marquee, loses focus, pinches, or loses its clipboard.
    for cancel in 0..4 {
        let mut rig = Rig::new(song(), None);
        set_clipboard(&rig.off.ctx, vec![(0.0, 0.5, points::Shape::default())]);
        rig.run(vec![egui::Event::PointerMoved(at)]);
        rig.button(at, true);
        for _ in 0..20 {
            rig.run(vec![]);
        }
        assert!(rig.ed.paste.is_some());
        match cancel {
            0 => rig.run(vec![egui::Event::PointerMoved(at + vec2(100.0, 50.0))]),
            1 => rig.run(vec![egui::Event::WindowFocused(false)]),
            2 => {
                rig.touch(1, egui::TouchPhase::Start, at);
                rig.touch(2, egui::TouchPhase::Start, at + vec2(100.0, 50.0));
            }
            _ => {
                set_clipboard(&rig.off.ctx, Vec::new());
                rig.run(vec![]);
            }
        }
        assert!(rig.ed.paste.is_none(), "cancel {cancel}");
        assert_eq!(rig.take(), [], "cancel {cancel}");
    }
}

#[test]
fn a_ruler_tap_seeks_before_the_content_end_and_stops_while_playing() {
    let mut s = song();
    points::set_points(
        &mut s.channels[1].lanes[0].clips[0],
        VOL,
        vec![Point {
            tick: 128,
            value: 100,
            ..Default::default()
        }],
    );
    let mut rig = Rig::new(s, None);
    rig.ed.sel = vec![true];
    rig.click(pos2(306.0, 24.0));
    assert_eq!(rig.take(), [Action::SetPlayhead(256)]);
    assert_eq!(rig.ed.sel, [false], "a ruler tap hides the point selection");
    // Past the content end (the 4-beat loop): nothing.
    rig.click(pos2(818.0, 24.0));
    assert_eq!(rig.take(), []);
    // While playing anywhere: the tap stops (the app's playhead rule).
    rig.t.playing = true;
    rig.click(pos2(818.0, 24.0));
    let seek = region::seek_tick(Some(clip(&rig.song)), 768, false);
    assert_eq!(rig.take(), [Action::SetPlayhead(seek)]);
}

#[test]
fn the_sample_length_handle_sets_the_length_in_one_edit_on_release() {
    // 1 s at 44.1 kHz and 120 BPM: 2 beats, the end line at 306. Not stretched yet: the handle shows all the same.
    let one_s = || sample(44_100, vec![0.5; 44_100], None);
    let mut rig = Rig::new(song(), Some(one_s()));
    let handle = rig.ed.handle.expect("the handle");
    // One beat right: L = 3 beats, one edit at the release (none while dragged); SYNC goes on.
    rig.drag(handle.center(), handle.center() + vec2(128.0, 40.0));
    assert_eq!(
        rig.take(),
        [Action::Stretch {
            len_beats: 3.0,
            ratio: 1.5
        }]
    );
    let c = clip(&rig.song);
    assert_eq!((c.file.source_len_beats, c.loop_len_beats), (3.0, 4.0));
    assert_eq!(c.file.sample.as_ref().unwrap().file.stretch_mode, 1);
    // Away and back in one drag: the ratio is the slot's, nothing goes out.
    let h = rig.ed.handle.unwrap().center();
    assert_eq!(h, handle.center() + vec2(128.0, 0.0));
    rig.run(vec![egui::Event::PointerMoved(h)]);
    rig.button(h, true);
    rig.run(vec![egui::Event::PointerMoved(h + vec2(100.0, 0.0))]);
    rig.run(vec![egui::Event::PointerMoved(h)]);
    rig.button(h, false);
    assert_eq!(rig.take(), []);
    // Far left: 0.2 × the 2-beat source.
    let handle = rig.ed.handle.unwrap();
    rig.drag(handle.center(), pos2(60.0, 194.0));
    assert_eq!(
        rig.take(),
        [Action::Stretch {
            len_beats: 0.4,
            ratio: 0.2
        }]
    );
    // Pressed and let go where it is: nothing.
    let handle = rig.ed.handle.unwrap();
    rig.click(handle.center());
    assert_eq!(rig.take(), []);
}

#[test]
fn the_waveform_draws_a_column_a_pixel_over_the_sample_and_caches_them() {
    // 1000 frames at 1 kHz, 120 BPM: 2 beats, 256 columns from x 50.
    let mono = sample(
        1000,
        (0..1000).map(|i| (i as f32 / 500.0) - 1.0).collect(),
        None,
    );
    let mut rig = Rig::new(song(), Some(mono));
    let wave = wave_color(&rig.song);
    let cols = rig.quads(wave);
    assert_eq!(cols.len(), 256);
    assert_eq!((cols[0].left(), cols[255].right()), (50.0, 306.0));
    // Mono, centred in the value area: the ramp's last column reaches near the top.
    assert!(cols.iter().all(|c| c.top() >= 64.0 && c.bottom() <= 324.0));
    rig.run(vec![]);
    rig.run(vec![]);
    assert_eq!(rig.ed.peaks.misses, 1, "the same view reuses its columns");
    // Stereo: two halves.
    let stereo = sample(1000, vec![0.5; 1000], Some(vec![-0.5; 1000]));
    let rig = Rig::new(song(), Some(stereo));
    let cols = rig.quads(wave);
    assert_eq!(cols.len(), 512);
    assert!(cols.iter().any(|c| c.bottom() <= 194.0));
    assert!(cols.iter().any(|c| c.top() >= 194.0));
    // Reversed: the ramp falls.
    let mut s = song();
    slot(&mut s).file.reverse = true;
    let mono = sample(
        1000,
        (0..1000).map(|i| (i as f32 / 500.0) - 1.0).collect(),
        None,
    );
    let rig = Rig::new(s, Some(mono));
    let cols = rig.quads(wave);
    assert!(cols[0].top() < cols[255].top());
}

#[test]
fn a_silent_stretch_draws_a_1_px_line_a_column() {
    // A column with less than 1 px of range is drawn as a 1 px line. 1000 silent frames: 256 columns from x 50, each
    // a 1 px line at the value area's centre, y 194.
    let rig = Rig::new(song(), Some(sample(1000, vec![0.0; 1000], None)));
    let wave = wave_color(&rig.song);
    // What the GPU gets: 256 px² in the waveform's colour. (egui draws a 1 px wide `rect_filled` as a line segment,
    // which shrinks a 1 px tall one to nothing.)
    let prims = rig.off.ctx.tessellate(rig.shapes.clone(), 1.0);
    let meshes = prims.iter().filter_map(|p| match &p.primitive {
        egui::epaint::Primitive::Mesh(m) => Some(m),
        egui::epaint::Primitive::Callback(_) => None,
    });
    let triangles = meshes.flat_map(|m| {
        (m.indices.as_chunks::<3>().0.iter())
            .map(|t| [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]))
    });
    let area: f32 = triangles
        .filter(|t| t.iter().all(|v| v.color == wave))
        .map(|[a, b, c]| {
            ((b.pos - a.pos).x * (c.pos - a.pos).y - (c.pos - a.pos).x * (b.pos - a.pos).y).abs()
        })
        .sum::<f32>()
        / 2.0;
    assert_eq!(area, 256.0);
    let line = |i: usize| Rect::from_min_size(pos2(50.0 + i as f32, 194.0), Vec2::splat(1.0));
    assert_eq!(rig.quads(wave), (0..256).map(line).collect::<Vec<_>>());
}

#[test]
fn the_draw_order_layers_marks_lanes_overlay_and_markers() {
    let mut s = song();
    // A VOL point (another lane, once PAN is current) and a song loop over beats 1…2.
    points::set_points(
        &mut s.channels[1].lanes[0].clips[0],
        VOL,
        vec![Point {
            tick: 0,
            value: 65535,
            ..Default::default()
        }],
    );
    s.play_loop = Some((128, 256));
    let mut rig = Rig::new(s, Some(sample(1000, vec![0.1; 1000], None)));
    rig.ed.lane = 1;
    rig.run(vec![]);
    let theme = crate::theme::theme_by_id(0);
    let c = &theme.colors;
    // Past the 4-beat loop, black at 15 % over the value area.
    let overlay = rig.rects(Color32::from_black_alpha(38));
    assert_eq!(
        overlay,
        [Rect::from_min_max(pos2(562.0, 64.0), pos2(1170.0, 324.0))]
    );
    // PAN: the 1 px centre line.
    let control = rig.rects(c.control_fill.to_color32());
    assert!(control.contains(&Rect::from_min_max(pos2(50.0, 194.0), pos2(1170.0, 195.0))));
    // The sample's end (2 beats) in menuFill; the song loop's edges in the channel colour.
    let menu = rig.rects(c.menu_fill.to_color32());
    assert!(menu.contains(&Rect::from_min_max(pos2(306.0, 48.0), pos2(307.0, 340.0))));
    let ink = channel_color(rig.song.channels[1].hue, &theme).to_color32();
    let loops = rig.rects(ink);
    for x in [178.0, 306.0] {
        assert!(
            loops.contains(&Rect::from_min_max(pos2(x, 48.0), pos2(x + 1.0, 340.0))),
            "{loops:?}"
        );
    }
    // VOL's curve in menuFill, full width at its one value.
    let vol_y = frame().point_y(65535, 65535);
    assert!(rig.shapes.iter().any(|s| matches!(&s.shape,
        Shape::LineSegment { points, stroke } if stroke.color == c.menu_fill.to_color32()
            && points[0].y == vol_y && points[1].x == 1170.0)));
}

// ---- the view in the app's slot ----

fn user(relative: &str) -> ContentPath {
    ContentPath::Shorthand {
        root: ContentRoot::User,
        relative: relative.into(),
    }
}

fn press(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

/// The view as the app runs it: its slot state and an `AppCtx` each frame.
struct AppRig {
    off: Offscreen,
    song: Song,
    audio: AudioLink,
    state: views::audio_editor::State,
    settings: Settings,
    files: ProjectFiles,
}

impl AppRig {
    /// One frame: what it painted and the edit it reported.
    fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<ClippedShape>, Option<SongEdit>) {
        let Self {
            off,
            song,
            audio,
            state,
            settings,
            files,
        } = self;
        let mut edit = None;
        let shapes = off.frame(events, &mut |ui, cx| {
            let mut selected = 1;
            let mut app = AppCtx {
                cx: UiCtx {
                    theme: cx.theme,
                    m: cx.m,
                },
                regions: Regions::new(SCREEN, cx.m),
                song,
                audio,
                selected: &mut selected,
                snapshot: None,
                playing: false,
                armed: false,
                previewing: false,
                middle_c_is_c4: false,
                settings,
                ghost_notes: true,
                files,
                edit: None,
                answer: None,
                covered: false,
                playhead: None,
                seek: None,
                midi_devices: &[],
                midi_rescan: false,
            };
            assert_eq!(views::audio_editor::show(state, ui, &mut app, 1), None);
            edit = app.edit;
        });
        (shapes, edit)
    }
}

#[test]
fn a_clip_on_a_project_made_wav_shows_its_waveform_and_a_vol_drag_is_one_undo_step() {
    let mut files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    let tone: Vec<f32> = (0..22_050).map(|i| (i as f32 * 0.05).sin() * 0.8).collect();
    let bytes = honeydew_audio::wav_writer::float32_wav(44_100, &[tone]);
    files.keep_user_file(Path::new("My Samples/tone.wav"), bytes);
    let mut song = song();
    slot(&mut song).file.path = Some(user("My Samples/tone.wav"));
    let mut rig = AppRig {
        off: Offscreen::new(),
        song,
        audio: AudioLink::default(),
        // On the clip, as the app opens it (`Slots::follow`).
        state: views::audio_editor::State {
            clips: [(1, (0, 1))].into(),
            ..Default::default()
        },
        settings: Settings::default(),
        files,
    };
    let (shapes, edit) = rig.frame(vec![]);
    assert_eq!(edit, None);
    let wave = wave_color(&rig.song);
    let quads = mesh_quads(shapes.iter().map(|s| &s.shape));
    let columns = quads.iter().filter(|q| q.1 == wave).count();
    // Half a second at 120 BPM: one beat, 128 columns.
    assert_eq!(columns, 128);
    // A tap adds a VOL point (one edit); a drag on it moves it: one final edit, at the release only.
    let f = Frame::new(Regions::new(SCREEN, &metrics()).editor, &metrics());
    let at = pos2(f.grid.left() + 128.0, f.value.center().y);
    let edits: Vec<_> = [
        vec![egui::Event::PointerMoved(at)],
        vec![press(at, true)],
        vec![press(at, false)],
    ]
    .into_iter()
    .map(|events| rig.frame(events).1)
    .collect();
    assert_eq!(edits, [None, None, Some(SongEdit::Final)]);
    let p = pos2(
        at.x,
        f.point_y(points::points(clip(&rig.song), VOL)[0].value, 65535),
    );
    let mut edits = vec![
        rig.frame(vec![egui::Event::PointerMoved(p)]).1,
        rig.frame(vec![press(p, true)]).1,
    ];
    for k in 1..=5 {
        let q = p + vec2(20.0 * k as f32, -10.0 * k as f32);
        edits.push(rig.frame(vec![egui::Event::PointerMoved(q)]).1);
    }
    edits.push(rig.frame(vec![press(p + vec2(100.0, -50.0), false)]).1);
    assert_eq!(edits.iter().flatten().count(), 1, "{edits:?}");
    assert_eq!(edits.last(), Some(&Some(SongEdit::Final)));
    let moved = points::points(clip(&rig.song), VOL);
    assert_eq!(moved.len(), 1);
    assert!(moved[0].tick > 128);
}

#[test]
fn the_right_edge_handle_scales_two_points_and_delete_removes_the_selection() {
    let mut s = song();
    let v = 30000;
    let pts = vec![
        Point {
            tick: 128,
            value: v,
            ..Default::default()
        },
        Point {
            tick: 256,
            value: v,
            ..Default::default()
        },
    ];
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts.clone());
    let mut rig = Rig::new(s, None);
    rig.ed.sel = vec![true, true];
    rig.run(vec![]);
    // 1 U right of the box, mid-height; a beat to the right doubles the span.
    let handle = pos2(306.0 + 32.0, frame().point_y(v, 65535));
    rig.drag(handle, handle + vec2(128.0, 0.0));
    let (want, _) = points::scaled(&pts, &[true, true], 128.0, snap()).unwrap();
    assert_eq!(want[1].tick, 384);
    assert_eq!(
        rig.take(),
        [Action::Points {
            key: VOL,
            points: want.clone()
        }]
    );
    // A tap on the first point alone: it is the selection, with the floating row; its `Delete`.
    rig.click(pos2(306.0 + 300.0, 200.0));
    rig.click(pos2(178.0, frame().point_y(v, 65535)));
    assert_eq!(rig.ed.sel, [true, false]);
    let delete = rig
        .off
        .ctx
        .read_response(egui::Id::new(("audio_point_button", 1usize)));
    rig.click(delete.expect("the floating row").rect.center());
    assert_eq!(
        rig.take(),
        [Action::Points {
            key: VOL,
            points: want[1..].to_vec()
        }]
    );
}

#[test]
fn a_scroll_makes_new_columns() {
    let mut s = song();
    s.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
    let mut rig = Rig::new(s, Some(sample(1000, vec![0.2; 16_000], None)));
    rig.run(vec![]);
    assert_eq!(rig.ed.peaks.misses, 1);
    // A drag on the ruler band scrolls (the touch x zone): new columns.
    rig.drag(pos2(600.0, 24.0), pos2(400.0, 24.0));
    assert_eq!(rig.take(), []);
    assert!(rig.ed.peaks.misses > 1);
    // 200 points, and the release frame's fling step (0.9 of the last move's 50).
    assert_eq!(rig.ed.views[&AT].left, 245.0 / 128.0);
}

#[test]
fn a_tap_on_a_point_selects_it_moves_nothing_and_another_tap_deselects() {
    let mut s = song();
    // Off the snap grid: a drag would snap it, a tap must not.
    let pts = vec![Point {
        tick: 100,
        value: 30000,
        ..Default::default()
    }];
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts.clone());
    let mut rig = Rig::new(s, None);
    let p = pos2(150.0, frame().point_y(30000, 65535));
    rig.click(p);
    assert_eq!(rig.take(), []);
    assert_eq!(rig.ed.sel, [true]);
    rig.click(p);
    assert_eq!(rig.take(), []);
    assert_eq!(rig.ed.sel, [false]);
    assert_eq!(points::points(clip(&rig.song), VOL), pts);
}

/// Where the text `label` was painted: its centre.
fn text_at(shapes: &[ClippedShape], label: &str) -> Pos2 {
    (shapes.iter())
        .find_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.text() == label => {
                Some(t.pos + t.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{label} not painted"))
}

/// The rects of `fill` that span the grid's height (marks and markers).
fn full_height(rig: &Rig, fill: Color32) -> Vec<Rect> {
    let g = frame().grid;
    (rig.rects(fill).into_iter())
        .filter(|r| r.top() == g.top() && r.bottom() == g.bottom() && r.left() >= g.left())
        .collect()
}

#[test]
fn zoom_speed_reverse_stretch_and_width_each_make_new_columns() {
    let mut rig = Rig::new(song(), Some(sample(1000, vec![0.3; 1000], None)));
    rig.run(vec![]);
    assert_eq!(rig.ed.peaks.misses, 1);
    let changed = |rig: &mut Rig, what: &str, edit: &dyn Fn(&mut Rig)| {
        let before = rig.ed.peaks.misses;
        edit(rig);
        rig.run(vec![]);
        assert!(rig.ed.peaks.misses > before, "{what}");
        let after = rig.ed.peaks.misses;
        rig.run(vec![]);
        assert_eq!(rig.ed.peaks.misses, after, "{what}: then cached");
    };
    changed(&mut rig, "speed", &|r| slot(&mut r.song).params[2] = 0.75);
    changed(&mut rig, "reverse", &|r| {
        slot(&mut r.song).file.reverse = true
    });
    changed(&mut rig, "SYNC", &|r| {
        let s = slot(&mut r.song);
        (s.file.stretch_mode, s.file.stretch_ratio) = (1, 2.0);
    });
    changed(&mut rig, "zoom", &|r| {
        r.run(vec![egui::Event::PointerMoved(pos2(600.0, 200.0))]);
        r.run(vec![egui::Event::Zoom(0.5)]);
    });
    changed(&mut rig, "width", &|r| r.rect.max.x = 1000.0);
}

#[test]
fn the_editor_keeps_no_replaced_sample_alive() {
    let mut rig = Rig::new(song(), Some(sample(1000, vec![0.3; 1000], None)));
    let frames = Arc::downgrade(&rig.sample.as_ref().unwrap().data.left);
    rig.sample = None;
    rig.run(vec![]);
    assert!(frames.upgrade().is_none());
}

#[test]
fn a_ruler_range_is_the_play_loop_and_its_menu_carries_the_playhead() {
    let mut rig = Rig::new(song(), None);
    // The transport at beat 3, outside the range.
    rig.t.tick = 384;
    let (a, b) = (pos2(178.0, 24.0), pos2(306.0, 24.0));
    rig.run(vec![egui::Event::PointerMoved(a)]);
    rig.button(a, true);
    for _ in 0..20 {
        rig.run(vec![]);
    }
    rig.run(vec![egui::Event::PointerMoved(b)]);
    rig.button(b, false);
    assert!(rig.take().contains(&Action::PlayLoop(Some((128, 256)))));
    assert_eq!(rig.song.play_loop, Some((128, 256)));
    // The `+` under the released edge opens the menu; `Clear`.
    rig.click(pos2(306.0, 48.0 + 20.0));
    rig.run(vec![]);
    let clear = text_at(&rig.shapes, "Clear");
    rig.click(clear);
    let range = TimeRange {
        start: 128,
        end: 256,
    };
    assert_eq!(
        rig.take(),
        [Action::Region {
            key: VOL,
            op: RegionOp::Clear,
            range,
            playhead: 384
        }]
    );
}

#[test]
fn a_ruler_tap_seeks_anywhere_with_a_loop_length_of_0() {
    let mut s = song();
    s.channels[1].lanes[0].clips[0].loop_len_beats = 0.0;
    let mut rig = Rig::new(s, None);
    rig.click(pos2(818.0, 24.0));
    assert_eq!(rig.take(), [Action::SetPlayhead(768)]);
}

#[test]
fn at_speed_plus_12_the_waveform_and_its_end_take_half_the_beats() {
    let mut s = song();
    slot(&mut s).params[2] = 0.75;
    let rig = Rig::new(s, Some(sample(1000, vec![0.3; 1000], None)));
    let cols = rig.quads(wave_color(&rig.song));
    assert_eq!(cols.len(), 128);
    assert_eq!(cols.last().map(|c| c.right()), Some(178.0));
    let theme = crate::theme::theme_by_id(0);
    let end = full_height(&rig, theme.colors.menu_fill.to_color32());
    assert_eq!(
        end,
        [Rect::from_min_max(pos2(178.0, 48.0), pos2(179.0, 340.0))]
    );
}

#[test]
fn vol_has_no_centre_line_and_the_marks_keep_2_4_u_apart() {
    let rig = Rig::new(song(), None);
    let theme = crate::theme::theme_by_id(0);
    let control = theme.colors.control_fill.to_color32();
    let centre = Rect::from_min_max(pos2(50.0, 194.0), pos2(1170.0, 195.0));
    assert!(!rig.rects(control).contains(&centre));
    // 4 U a beat: beats (2 px) and bars (4 px), no quarter beats (1 U apart).
    let marks = full_height(&rig, control);
    assert!(marks.contains(&Rect::from_min_max(pos2(177.0, 48.0), pos2(179.0, 340.0))));
    assert!(marks.iter().all(|r| r.width() >= 2.0), "{marks:?}");
}

#[test]
fn markers_on_the_grid_edges_are_not_drawn() {
    // The song loop's edges at the grid's left (beat 0) and right (8.75 beats) edges, the sample past the grid.
    let mut s = song();
    s.play_loop = Some((0, 1120));
    let rig = Rig::new(s, Some(sample(1000, vec![0.3; 10_000], None)));
    let theme = crate::theme::theme_by_id(0);
    let ink = channel_color(rig.song.channels[1].hue, &theme).to_color32();
    assert_eq!(full_height(&rig, ink), []);
    let menu = full_height(&rig, theme.colors.menu_fill.to_color32());
    assert_eq!(menu, []);
}

#[test]
fn the_shared_long_press_time_and_calm_zone_decide_a_touch_on_empty_grid() {
    // `gesture::LONG_PRESS_S` and `CALM_U` (0.3 s; 0.5 U, 16 points at U = 32): released at 0.28 s, or moved 0.45 U, a
    // press on empty grid is a tap (a point); at 0.32 s it is a long press, 0.55 U out a drag: nothing.
    let points_after = |secs: f64, by: f32| {
        let mut rig = Rig::new(song(), None);
        let p = pos2(600.0, 200.0);
        rig.run(vec![egui::Event::PointerMoved(p)]);
        rig.button(p, true);
        let mut to = p;
        if by != 0.0 {
            to = p + vec2(by, 0.0);
            rig.run(vec![egui::Event::PointerMoved(to)]);
        }
        // Each frame is 1/60 s on.
        let held = if by != 0.0 { 1.0 / 60.0 } else { 0.0 };
        rig.off.time += secs - held - 1.0 / 60.0;
        rig.button(to, false);
        points::points(clip(&rig.song), VOL).len()
    };
    assert_eq!(points_after(0.28, 0.0), 1);
    assert_eq!(points_after(0.32, 0.0), 0);
    assert_eq!(points_after(0.2, 0.45 * 32.0), 1);
    assert_eq!(points_after(0.2, 0.55 * 32.0), 0);
}

#[test]
fn a_point_or_the_stretch_handle_moved_inside_the_calm_zone_is_a_tap_not_a_drag() {
    // 0.3 U (9.6 points at U = 32) is past egui's 6-point click distance but inside the 0.5 U calm zone: no drag.
    let by = 0.3 * 32.0;
    let pts = vec![Point {
        tick: 128,
        value: 40000,
        ..Default::default()
    }];
    let mut s = song();
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts.clone());
    let mut rig = Rig::new(s, None);
    assert_eq!(rig.ed.sel, [false]);
    let p = pos2(178.0, frame().point_y(40000, 65535));
    rig.drag(p, p + vec2(0.0, by));
    // A tap on the point: it is unchanged, and selected.
    assert_eq!(rig.take(), []);
    assert_eq!(points::points(clip(&rig.song), VOL), pts);
    assert_eq!(rig.ed.sel, [true]);
    // The stretch handle (a SYNC slot, 1 s at 120 BPM: 2 beats) sets no length, even with Snap off (a drag would set
    // 2 + 9.6/128 beats).
    let mut s = song();
    slot(&mut s).file.stretch_mode = 1;
    let mut rig = Rig::new(s, Some(sample(44_100, vec![0.5; 44_100], None)));
    rig.ed.snap.on = false;
    let handle = rig.ed.handle.expect("the handle").center();
    rig.drag(handle, handle + vec2(by, 0.0));
    assert_eq!(rig.take(), []);
}

#[test]
fn a_press_released_as_its_300_ms_come_due_is_a_long_press() {
    let mut rig = Rig::new(song(), None);
    let p = pos2(600.0, 200.0);
    rig.run(vec![egui::Event::PointerMoved(p)]);
    rig.button(p, true);
    for _ in 0..10 {
        rig.run(vec![]);
    }
    // No frame between: the release frame is the first past 300 ms.
    rig.off.time += 0.2;
    rig.button(p, false);
    assert_eq!(rig.take(), []);
    assert!(points::points(clip(&rig.song), VOL).is_empty());
}

/// The automation editor and the audio editor over [`RECT`], one context (one app-wide clipboard).
struct Both {
    off: Offscreen,
    song: Song,
    auto: automation::AutomationEditor,
    audio: AudioEditor,
    key: crate::ui::preview::ClipKey,
    shapes: Vec<ClippedShape>,
    actions: Vec<Action>,
}

impl Both {
    /// One frame of the audio editor (`audio`) or the automation editor; the audio editor's edits apply.
    fn run(&mut self, audio: bool, events: Vec<egui::Event>) {
        let Self {
            off,
            song,
            auto,
            audio: ed,
            key,
            ..
        } = self;
        let mut actions = Vec::new();
        self.shapes = off.frame(events, &mut |ui, cx| {
            if audio {
                let mut ui = ui.new_child(UiBuilder::new().max_rect(RECT));
                actions = ed.show(&mut ui, cx, song, AT, None, Transport::default());
            } else {
                auto.show(ui, cx, RECT, song, *key, &automation::Options::default());
            }
        });
        for a in &actions {
            apply(song, AT, a);
        }
        self.actions.extend(actions);
    }

    /// A tap at `p`, or a long press (`hold`); then a frame for the floating row to lay out.
    fn tap(&mut self, audio: bool, p: Pos2, hold: bool) {
        self.run(audio, vec![egui::Event::PointerMoved(p)]);
        self.run(audio, vec![press(p, true)]);
        for _ in 0..if hold { 20 } else { 0 } {
            self.run(audio, vec![]);
        }
        self.run(audio, vec![press(p, false)]);
        self.run(audio, vec![]);
    }

    fn automation(&self) -> Vec<Point> {
        let clip = &self.song.channels[0].lanes[self.key.lane].clips[0];
        points::points(clip, PointKey::Automation)
    }
}

#[test]
fn a_copy_in_the_automation_editor_pastes_in_an_envelope_lane_and_back() {
    use crate::song::AutomationTarget;
    // The master's Pan automated (points at beats 0 and 2) beside the audio clip.
    let mut song = song();
    let target = AutomationTarget {
        instance_id: 0,
        param: 1,
    };
    let ch = &mut song.channels[0];
    ch.add_automation_lane(target);
    let lane = (ch.lanes.iter())
        .position(|l| l.target == Some(target))
        .unwrap();
    let clip = ch.lanes[lane].add_clip(0, 4.0);
    let auto = vec![
        Point {
            tick: 0,
            value: 0,
            ..Default::default()
        },
        Point {
            tick: 256,
            value: 32767,
            ..Default::default()
        },
    ];
    points::set_points(clip, PointKey::Automation, auto.clone());
    let key = crate::ui::preview::ClipKey {
        channel: 0,
        lane,
        id: clip.id,
    };
    let mut both = Both {
        off: Offscreen::new(),
        song,
        auto: Default::default(),
        audio: Default::default(),
        key,
        shapes: Vec::new(),
        actions: Vec::new(),
    };
    for _ in 0..3 {
        both.run(false, vec![]);
    }
    // The automation editor: a tap on the beat-2 point, then its `Copy`.
    let f = frame();
    both.tap(
        false,
        pos2(50.0 + 256.0, f.row_y(points::row(32767, 65534))),
        false,
    );
    both.tap(false, text_at(&both.shapes, "Copy"), false);
    let copied = automation::clipboard(&both.off.ctx);
    assert_eq!(copied, [(2.0, 32767.0 / 65534.0, points::Shape::default())]);

    // The audio editor's VOL lane: a long press shows `Paste`; its tap pastes the copied point (full scale kept).
    both.tap(true, pos2(572.0, 200.0), true);
    let menu = egui::Id::new("audio_editor").with("paste");
    let paste = crate::ui::insert::button(&both.off.ctx, menu, 0).expect("Paste");
    both.tap(true, paste.center(), false);
    let (vol, _) = points::paste(&[], &copied, 522.0, snap(), 65535);
    assert_eq!(
        vol,
        [Point {
            tick: 512,
            value: 32768,
            ..Default::default()
        }]
    );
    assert_eq!(
        both.actions,
        [Action::Points {
            key: VOL,
            points: vol.clone()
        }]
    );

    // Back: the envelope point's `Copy` (deselect, select it), then `Paste` in the automation editor.
    both.tap(true, pos2(700.0, 300.0), false);
    both.tap(true, pos2(562.0, f.row_y(points::row(32768, 65535))), false);
    let copy = both
        .off
        .ctx
        .read_response(egui::Id::new(("audio_point_button", 0usize)));
    both.tap(true, copy.expect("the floating row").rect.center(), false);
    let copied = automation::clipboard(&both.off.ctx);
    assert_eq!(copied, [(4.0, 32768.0 / 65535.0, points::Shape::default())]);
    let finger = pos2(50.0 + 384.0, 300.0);
    both.tap(false, finger, true);
    let menu = egui::Id::new("automation_editor").with("paste");
    let paste = crate::ui::insert::button(&both.off.ctx, menu, 0).expect("Paste");
    both.tap(false, paste.center(), false);
    let (want, _) = points::paste(&auto, &copied, 384.0, snap(), 65534);
    assert_eq!(
        want[2],
        Point {
            tick: 384,
            value: 32767,
            ..Default::default()
        }
    );
    assert_eq!(both.automation(), want);
}

#[test]
fn the_audio_editors_wheel_zooms_the_time_on_the_ruler_and_on_the_grid() {
    // The zoom-x zone covers both and comes first.
    let mut s = song();
    s.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
    let mut rig = Rig::new(s, None);
    let f = frame();
    for at in [f.time.center(), f.grid.center()] {
        let before = rig.ed.views[&AT];
        let beat = |v: View| v.left + f64::from((at.x - f.grid.left()) / v.beat_w);
        rig.run(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 200.0),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let after = rig.ed.views[&AT];
        assert!(
            (after.beat_w - before.beat_w * 1.25).abs() < 1e-3,
            "{at:?}: {after:?}"
        );
        assert!((beat(after) - beat(before)).abs() < 1e-4, "{at:?}");
    }
}

#[test]
fn at_its_opening_4_u_a_beat_the_ruler_labels_every_bar() {
    // Every ruler follows the same law: at 4 U a beat s2 is one bar, so bar 2 has its label, by the bar line 4 beats
    // (512 points at U = 32) right of bar 1's.
    let rig = Rig::new(song(), None);
    let ink = (crate::theme::theme_by_id(0).colors.text.to_color32()).gamma_multiply(0.8);
    let labels = Offscreen::texts(&rig.shapes);
    assert!(labels.contains(&("2".into(), ink)), "{labels:?}");
    assert!(labels.contains(&("3".into(), ink)), "{labels:?}");
    let (two, three) = (text_at(&rig.shapes, "2"), text_at(&rig.shapes, "3"));
    assert!(((three.x - two.x) - 512.0).abs() < 1.0, "{two:?} {three:?}");
}

#[test]
fn another_clip_takes_no_fling_from_the_last_one() {
    // A fling of one clip's time (on the ruler band) does not move the next clip's view.
    let mut s = song();
    s.channels[1].lanes[0].clips[0].loop_len_beats = 32.0;
    let lane = &mut s.channels[1].lanes[0];
    lane.add_clip(8192, 32.0);
    lane.clips[1].file.sample = Some(Box::default());
    let (first, second): (At, At) = (AT, (1, 0, 2));
    let (mut off, mut ed) = (Offscreen::new(), AudioEditor::default());
    let mut run = |ed: &mut AudioEditor, at: At, events| {
        off.frame(events, &mut |ui, cx| {
            let mut ui = ui.new_child(UiBuilder::new().max_rect(RECT));
            ed.show(&mut ui, cx, &s, at, None, Transport::default());
        });
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let from = pos2(600.0, 24.0);
    run(&mut ed, first, vec![egui::Event::PointerMoved(from)]);
    run(&mut ed, first, vec![button(from, true)]);
    for k in 1..=3 {
        let to = from - vec2(60.0 * k as f32, 0.0);
        run(&mut ed, first, vec![egui::Event::PointerMoved(to)]);
    }
    run(&mut ed, first, vec![button(from - vec2(180.0, 0.0), false)]);
    let flying = ed.views[&first].left;
    run(&mut ed, first, vec![]);
    assert_ne!(ed.views[&first].left, flying, "the first clip flings");
    for _ in 0..10 {
        run(&mut ed, second, vec![]);
    }
    assert_eq!(ed.views[&second].left, 0.0);
}

// ---- a touch the editor stops seeing, a press held over a focus loss and a pinch ----

/// A SYNC slot of a 1 s sample at 120 BPM (2 beats), Vol point (128, 40000): the rig, the point and the handle.
fn stretch_rig() -> (Rig, Pos2, Pos2) {
    let mut s = song();
    slot(&mut s).file.stretch_mode = 1;
    let pts = vec![Point {
        tick: 128,
        value: 40000,
        ..Default::default()
    }];
    points::set_points(&mut s.channels[1].lanes[0].clips[0], VOL, pts);
    let rig = Rig::new(s, Some(sample(44_100, vec![0.5; 44_100], None)));
    let point = pos2(178.0, frame().point_y(40000, 65535));
    let handle = rig.ed.handle.expect("the handle").center();
    (rig, point, handle)
}

impl Rig {
    /// Pressed at `from` and moved a beat right and a bit down, still held.
    fn hold_dragged(&mut self, from: Pos2) -> Pos2 {
        self.run(vec![egui::Event::PointerMoved(from)]);
        self.button(from, true);
        let to = from + vec2(128.0, 20.0);
        self.run(vec![egui::Event::PointerMoved(from + vec2(64.0, 10.0))]);
        self.run(vec![egui::Event::PointerMoved(to)]);
        to
    }

    fn touch(&mut self, id: u64, phase: egui::TouchPhase, pos: Pos2) {
        self.run(vec![egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(id),
            phase,
            pos,
            force: None,
        }]);
    }
}

#[test]
fn a_drag_the_editor_stops_seeing_ends_with_no_edit() {
    for on_handle in [false, true] {
        let (mut rig, point, handle) = stretch_rig();
        let to = rig.hold_dragged(if on_handle { handle } else { point });
        // Hidden (another view took the editor's place), let go there, shown again: nothing commits.
        rig.ed.end_touch();
        rig.button(to, false);
        rig.run(vec![]);
        assert_eq!(rig.take(), [], "handle {on_handle}");
        assert_eq!(
            rig.ed.handle.unwrap().center(),
            handle,
            "handle {on_handle}"
        );
    }
}

#[test]
fn a_press_held_over_a_focus_loss_adds_no_point_and_stretches_nothing() {
    let lost = |rig: &mut Rig| {
        rig.run(vec![egui::Event::WindowFocused(false)]);
        rig.run(vec![egui::Event::WindowFocused(true)]);
    };
    // On empty grid: no tap's point.
    let (mut rig, ..) = stretch_rig();
    let empty = pos2(700.0, 300.0);
    rig.run(vec![egui::Event::PointerMoved(empty)]);
    rig.button(empty, true);
    lost(&mut rig);
    rig.button(empty, false);
    // A point's drag and the handle's: nothing.
    for on_handle in [false, true] {
        let (mut rig, point, handle) = stretch_rig();
        let to = rig.hold_dragged(if on_handle { handle } else { point });
        lost(&mut rig);
        rig.button(to, false);
        rig.run(vec![]);
        assert_eq!(rig.take(), [], "handle {on_handle}");
    }
    assert_eq!(rig.take(), []);
}

#[test]
fn a_second_finger_cancels_a_stretch_drag() {
    use egui::TouchPhase::{End, Move, Start};
    let (mut rig, _, handle) = stretch_rig();
    let touch = |rig: &mut Rig, phase, pos: Pos2| {
        let pointer = match phase {
            Start => vec![egui::Event::PointerMoved(pos), press(pos, true)],
            Move => vec![egui::Event::PointerMoved(pos)],
            _ => vec![press(pos, false), egui::Event::PointerGone],
        };
        let touch = egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(1),
            phase,
            pos,
            force: None,
        };
        rig.run([vec![touch], pointer].concat());
    };
    touch(&mut rig, Start, handle);
    touch(&mut rig, Move, handle + vec2(64.0, 0.0));
    rig.touch(2, Start, pos2(900.0, 100.0));
    touch(&mut rig, Move, handle + vec2(128.0, 0.0));
    rig.touch(2, End, pos2(900.0, 100.0));
    touch(&mut rig, End, handle + vec2(128.0, 0.0));
    rig.run(vec![]);
    assert_eq!(rig.take(), []);
    assert_eq!(rig.ed.handle.unwrap().center(), handle);
}
