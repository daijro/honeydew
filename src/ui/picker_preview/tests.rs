use std::sync::Arc;

use eframe::egui::{pos2, vec2};
use honeydew_audio::sample::SampleData;

use super::*;

/// A row 600 points wide from x = 100.
fn row() -> Rect {
    Rect::from_min_size(pos2(100.0, 50.0), vec2(600.0, 40.0))
}

#[test]
fn the_note_is_24_at_the_left_edge_84_at_the_right_and_54_in_the_middle() {
    // note = trunc(24 + 60u), u = clamp((x − rowLeft)/rowWidth, 0, 1).
    let row = row();
    assert_eq!(note_at(100.0, row), 24);
    assert_eq!(note_at(400.0, row), 54);
    assert_eq!(note_at(700.0, row), 84);
    // Each note covers 1/60 of the width; 84 only at the right edge.
    assert_eq!(note_at(109.99, row), 24);
    assert_eq!(note_at(110.0, row), 25);
    assert_eq!(note_at(699.9, row), 83);
    // Clamped outside the row.
    assert_eq!(note_at(0.0, row), 24);
    assert_eq!(note_at(900.0, row), 84);
}

#[test]
fn a_finger_holds_its_note_until_it_lifts_and_moving_changes_nothing() {
    let mut a = Audition {
        on: true,
        ..Audition::default()
    };
    let at = |x| pos2(x, 60.0);
    assert!(!a.touch(&[Finger::Down(1, at(100.0))], Some(row())));
    assert_eq!(a.take_sounds(), [Sound::On(24)]);
    a.touch(&[Finger::Move(1, at(700.0))], Some(row()));
    assert_eq!(a.take_sounds(), []);
    a.touch(&[Finger::Up(1, at(700.0))], Some(row()));
    assert_eq!(a.take_sounds(), [Sound::Off(24)]);
}

#[test]
fn a_second_finger_while_one_holds_re_picks_and_sounds_nothing() {
    let mut a = Audition {
        on: true,
        ..Audition::default()
    };
    let at = |x| pos2(x, 60.0);
    a.touch(&[Finger::Down(1, at(400.0))], Some(row()));
    assert!(a.touch(&[Finger::Down(2, at(700.0))], Some(row())));
    assert_eq!(a.take_sounds(), [Sound::On(54)]);
    // Its lift is not the note's.
    a.touch(&[Finger::Up(2, at(700.0))], Some(row()));
    assert_eq!(a.take_sounds(), []);
    a.touch(&[Finger::Up(1, at(400.0))], Some(row()));
    assert_eq!(a.take_sounds(), [Sound::Off(54)]);
    // With no finger down the next touch plays again.
    assert!(!a.touch(&[Finger::Down(3, at(700.0))], Some(row())));
    assert_eq!(a.take_sounds(), [Sound::On(84)]);
}

#[test]
fn enter_holds_note_60_and_one_note_sounds_at_a_time() {
    let mut a = Audition {
        on: true,
        ..Audition::default()
    };
    a.touch(&[Finger::Down(1, pos2(100.0, 60.0))], Some(row()));
    // A new trigger first releases the note before.
    a.key(true, true);
    assert_eq!(
        a.take_sounds(),
        [Sound::On(24), Sound::Off(24), Sound::On(60)]
    );
    // A key repeat retriggers nothing; the finger's lift is not Enter's note.
    a.key(true, true);
    a.touch(&[Finger::Up(1, pos2(100.0, 60.0))], Some(row()));
    assert_eq!(a.take_sounds(), []);
    a.key(false, false);
    assert_eq!(a.take_sounds(), [Sound::Off(60)]);
}

#[test]
fn turning_it_off_releases_the_held_note_and_no_row_takes_no_touch() {
    let mut a = Audition {
        on: true,
        ..Audition::default()
    };
    a.touch(&[Finger::Down(1, pos2(400.0, 60.0))], Some(row()));
    a.off();
    assert!(!a.on);
    assert_eq!(a.take_sounds(), [Sound::On(54), Sound::Off(54)]);
    a.on = true;
    a.touch(&[Finger::Down(1, pos2(400.0, 60.0))], None);
    assert_eq!(a.take_sounds(), []);
}

#[test]
fn the_loading_fill_pulses_twice_a_second_up_to_half_alpha() {
    assert_eq!(pulse(0.0), 0.0);
    assert!((pulse(0.25) - 0.5).abs() < 1e-6);
    assert!(pulse(0.5).abs() < 1e-6);
    assert!((pulse(1.75) - 0.5).abs() < 1e-6);
}

fn wave(frames: usize) -> Waveform {
    Waveform::new(Arc::new(SampleData {
        sample_rate: 1000,
        left: vec![0.5; frames].into(),
        right: None,
        meta: Default::default(),
    }))
}

#[test]
fn a_sample_strip_plays_its_pick_from_the_start_once_decoded_and_stops_once() {
    let mut strip = SampleStrip::new("Kick".into(), Some(wave(100)));
    assert!(strip.playing());
    assert_eq!(strip.take_plays(), [Play::From(0)]);
    strip.stop();
    strip.stop();
    assert_eq!(strip.take_plays(), [Play::Stop]);
    // One that does not decode only shows its name.
    let mut none = SampleStrip::new("Broken".into(), None);
    assert!(!none.playing());
    assert_eq!(none.take_plays(), []);
}

#[test]
fn the_strips_waveform_is_one_smooth_band_of_the_peaks_a_device_pixel_a_column() {
    // #108: a column a device pixel, both channels in one band, ± the column's peak held 20 ms (the cycles of a low note
    // fill in) as a share of the file's peak. A 20 Hz tone at half scale fills the whole height everywhere; silence is a
    // 1 px line.
    let tone = |i: usize| 0.5 * (std::f32::consts::TAU * 20.0 * i as f32 / 1000.0).cos();
    for (left, full) in [(tone as fn(usize) -> f32, true), (|_| 0.0, false)] {
        let mut strip = SampleStrip::new("Kick".into(), Some(stereo(2000, left)));
        let panel = Rect::from_min_size(pos2(0.0, 0.0), vec2(390.0, 120.0));
        let mut off = crate::ui::offscreen::Offscreen::new();
        let mut wave_rect = Rect::NOTHING;
        let shapes = off.frame(vec![], &mut |ui, cx| {
            wave_rect = SampleStrip::layout(panel, cx)[2];
            strip.show(ui, cx, panel, Id::new("strip"));
        });
        let ink = crate::theme::theme_by_id(0)
            .colors
            .text
            .to_color32()
            .gamma_multiply(WAVE_ALPHA);
        let quads = crate::ui::audio_editor::mesh_quads(shapes.iter().map(|s| &s.shape));
        let cols: Vec<_> = quads
            .into_iter()
            .filter(|q| q.1 == ink)
            .map(|q| q.0)
            .collect();
        let (x0, width) = pixel_columns(wave_rect, &crate::ui::offscreen::metrics());
        assert!(width > 300, "{wave_rect:?}");
        assert_eq!(cols.len(), width);
        let height = if full { wave_rect.height() } else { 1.0 };
        for (i, c) in cols.iter().enumerate() {
            assert_eq!((c.left(), c.width()), (x0 + i as f32, 1.0));
            assert!(
                (c.height() - height).abs() <= 1.0,
                "{full}: column {i}: {c:?}"
            );
            assert!((c.center().y - wave_rect.center().y).abs() <= 0.5, "{c:?}");
        }
    }
    // A decaying low tone's band falls a little every column: the held crests join in ramps, no flat steps.
    let decay = |i: usize| {
        0.5 * (1.0 - i as f32 / 2000.0) * (std::f32::consts::TAU * 20.0 * i as f32 / 1000.0).cos()
    };
    let band = envelope(&stereo(2000, decay), 400);
    assert!(band.windows(2).all(|w| w[1] < w[0]), "{band:?}");
}

/// `frames` frames at 1 kHz: `left(i)` on the left, a tenth of it on the right.
fn stereo(frames: usize, left: fn(usize) -> f32) -> Waveform {
    Waveform::new(Arc::new(SampleData {
        sample_rate: 1000,
        left: (0..frames).map(left).collect::<Vec<_>>().into(),
        right: Some(
            (0..frames)
                .map(|i| left(i) / 10.0)
                .collect::<Vec<_>>()
                .into(),
        ),
        meta: Default::default(),
    }))
}

#[test]
fn the_progress_reads_minutes_and_seconds() {
    assert_eq!(clock(0.0), "0:00");
    assert_eq!(clock(9.9), "0:09");
    assert_eq!(clock(75.0), "1:15");
}
