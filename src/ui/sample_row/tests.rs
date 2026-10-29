use eframe::egui::{Pos2, Vec2, pos2, vec2};

use super::*;
use crate::metrics::DEFAULT_UI_SCALE;
use crate::song::ChannelKind;
use crate::ui::offscreen::{Offscreen, SCREEN};
use honeydew_audio::sample_edit::speed_semitones;

const U: f32 = 32.0;

#[test]
fn level_reads_66_v_to_the_0_6_minus_60_db_and_minus_inf_at_zero() {
    // VOL: v = .25, .5, .75, 1 → −31.3, −16.5, −4.5, +6.0 dB; `%.1f` has no plus sign.
    let cases = [
        (0.0, "Level: -inf."),
        (0.25, "Level: -31.3 dB"),
        (0.5, "Level: -16.5 dB"),
        (0.75, "Level: -4.5 dB"),
        (1.0, "Level: 6.0 dB"),
        // The default slot level is 0 dB; a value that prints as zero has no minus sign.
        (0.8531, "Level: 0.0 dB"),
    ];
    for (v, text) in cases {
        assert_eq!(level_text(v), text, "{v}");
    }
}

#[test]
fn pan_reads_center_or_the_percent_to_one_side() {
    let cases = [
        (0.5, "Pan: center"),
        (0.0, "Pan: 100% left"),
        (0.25, "Pan: 50% left"),
        (0.75, "Pan: 50% right"),
        (1.0, "Pan: 100% right"),
    ];
    for (v, text) in cases {
        assert_eq!(pan_text(v), text, "{v}");
    }
}

#[test]
fn speed_truncates_48v_minus_24_to_whole_semitones_and_reads_1x_at_zero() {
    let cases = [
        (0.5, "Speed: 1.0x"),
        // 0.9 of a semitone truncates to none, either way.
        (0.5 + 0.9 / 48.0, "Speed: 1.0x"),
        (0.5 - 0.9 / 48.0, "Speed: 1.0x"),
        (0.53, "Speed: +1 semitone"),
        (0.47, "Speed: -1 semitone"),
        (0.75, "Speed: +12 semitones"),
        (0.0, "Speed: -24 semitones"),
        (1.0, "Speed: +24 semitones"),
    ];
    for (v, text) in cases {
        assert_eq!(speed_text(v), text, "{v}");
    }
}

#[test]
fn speed_reads_the_semitones_the_engine_plays_on_every_grid_step() {
    // `speed_semitones` is the engine's law, trunc(fma_f32(v, 48, −24)): on each step v = f32(k/48) the text names the
    // semitones the pad plays, which is k − 24 except at k = 16, 19, 22 (−7, −4, −1) and k = 25, 28 … 40 (0, 3 … 15).
    for k in 0..=48u32 {
        let v = k as f32 / 48.0;
        let n = match k {
            16 | 19 | 22 => k as i32 - 23,
            25 | 28 | 31 | 34 | 37 | 40 => k as i32 - 25,
            _ => k as i32 - 24,
        };
        assert_eq!(speed_semitones(v), n, "the premise, k = {k}");
        let want = match n {
            0 => "Speed: 1.0x".to_owned(),
            1 | -1 => format!("Speed: {n:+} semitone"),
            _ => format!("Speed: {n:+} semitones"),
        };
        assert_eq!(speed_text(v), want, "k = {k}");
    }
}

#[test]
fn a_pitch_past_the_sliders_range_shows_at_its_end() {
    let mut slot = SamplerSlot::default();
    for (st, v) in [(30.0, 1.0), (24.0, 1.0), (-30.0, 0.0), (-12.0, 0.25)] {
        slot.file.pitch_semitones = st;
        assert_eq!(Row::of(Some(&slot)).pitch, v, "{st}");
    }
}

#[test]
fn pitch_rounds_48v_minus_24_and_signs_only_raised_pitches() {
    let cases = [
        (0.5, "Pitch: 0 semitones"),
        // lround: 0.9 of a semitone is one.
        (0.5 + 0.9 / 48.0, "Pitch: +1 semitone"),
        (0.5 - 0.9 / 48.0, "Pitch: -1 semitone"),
        (0.75, "Pitch: +12 semitones"),
        (0.0, "Pitch: -24 semitones"),
        (1.0, "Pitch: +24 semitones"),
    ];
    for (v, text) in cases {
        assert_eq!(pitch_text(v), text, "{v}");
    }
}

/// A slot with its own values everywhere the row looks.
fn slot() -> SamplerSlot {
    let mut slot = SamplerSlot::default();
    slot.params[..3].copy_from_slice(&[0.3, 0.25, 0.75]);
    slot.file.pitch_semitones = -12.0;
    slot.file.reverse = true;
    (slot.file.stretch_mode, slot.file.stretch_ratio) = (1, 1.5);
    slot
}

#[test]
fn the_row_shows_the_slots_params_pitch_reverse_and_sync() {
    let row = Row::of(Some(&slot()));
    assert_eq!(
        row,
        Row {
            level: 0.3,
            pan: 0.25,
            speed: 0.75,
            pitch: 0.25,
            reverse: true,
            sync: true
        }
    );
    let values: Vec<f32> = CTLS[1..].iter().map(|&c| row.get(c)).collect();
    assert_eq!(values, [0.3, 0.25, 0.75, 0.25, 1.0, 1.0]);
    // Any nonzero mode is on.
    let mut odd = slot();
    odd.file.stretch_mode = 7;
    assert!(Row::of(Some(&odd)).sync);
    // No slot: the default slot's values (0 dB, centre, 1.0x, no pitch, both off).
    let none = Row::of(None);
    assert_eq!(none, Row::of(Some(&SamplerSlot::default())));
    assert_eq!(
        (none.level, none.pan, none.speed, none.pitch),
        (0.8531, 0.5, 0.5, 0.5)
    );
    assert!(!none.reverse && !none.sync);
}

#[test]
fn each_control_writes_its_own_field_only() {
    let base = slot();
    let after = |ctl, v| {
        let mut slot = base.clone();
        set(&mut slot, ctl, v);
        slot
    };
    let mut want = base.clone();
    want.params[0] = 0.9;
    assert_eq!(after(Ctl::Level, 0.9), want);
    want = base.clone();
    want.params[1] = 0.6;
    assert_eq!(after(Ctl::Pan, 0.6), want);
    want = base.clone();
    want.params[2] = 0.1;
    assert_eq!(after(Ctl::Speed, 0.1), want);
    // Pitch: STR1 semitones, lround(48v − 24).
    want = base.clone();
    want.file.pitch_semitones = 12.0;
    assert_eq!(after(Ctl::Pitch, 0.75), want);
    want.file.pitch_semitones = 1.0;
    assert_eq!(after(Ctl::Pitch, 0.5 + 0.9 / 48.0), want);
    // REV: the reverse flag.
    want = base.clone();
    want.file.reverse = false;
    assert_eq!(after(Ctl::Rev, 0.0), want);
    // SYNC: the mode, 0 or 1; the ratio stays as stored (a later SYNC-on uses it).
    want = base.clone();
    want.file.stretch_mode = 0;
    assert_eq!(after(Ctl::Sync, 0.0), want);
    let mut off = base.clone();
    off.file.stretch_mode = 0;
    set(&mut off, Ctl::Sync, 1.0);
    assert_eq!((off.file.stretch_mode, off.file.stretch_ratio), (1, 1.5));
    // The icon button has no value.
    assert_eq!(after(Ctl::Pick, 1.0), base);
}

/// Master (0), Audio Clip (1) with clips 1 and 2 on its main lane, MiniSynth (2) with a clip.
fn song() -> Song {
    let mut song = Song::default();
    song.add_channel(ChannelKind::AudioClip);
    song.add_channel(ChannelKind::Synth(crate::song::SynthKind::MiniSynth));
    for start in [0, 768] {
        song.channels[1].lanes[0].add_clip(start, 4.0);
    }
    song.channels[2].lanes[0].add_clip(0, 4.0);
    song
}

fn key(channel: usize, lane: usize, id: u32) -> ClipKey {
    ClipKey { channel, lane, id }
}

#[test]
fn the_current_clip_is_the_audio_editors_else_the_main_lanes_current_one() {
    let mut song = song();
    let mut editor = audio_editor::State::default();
    // No current clip: `DESc`@24 is 0.
    assert_eq!(current(&song, 1, &editor, false), None);
    song.channels[1].lanes[0].file.current_clip_id = 2;
    assert_eq!(current(&song, 1, &editor, false), Some(key(1, 0, 2)));
    // A current clip id that names no clip.
    song.channels[1].lanes[0].file.current_clip_id = 9;
    assert_eq!(current(&song, 1, &editor, false), None);
    // The audio editor's clip wins, on any audio lane of the channel.
    let more = song.channels[1].add_lane(LaneKind::Audio);
    song.channels[1].lanes[more].add_clip(0, 2.0);
    editor.clips.insert(1, (more, 1));
    assert_eq!(current(&song, 1, &editor, false), Some(key(1, more, 1)));
    editor.clips.insert(1, (0, 1));
    assert_eq!(current(&song, 1, &editor, false), Some(key(1, 0, 1)));
    // Only while the channel's main lane is audio: not on a synth, nor the master, nor a missing channel.
    song.channels[2].lanes[0].file.current_clip_id = 1;
    editor.clips.insert(2, (0, 1));
    for channel in [0, 2, 7] {
        assert_eq!(current(&song, channel, &editor, false), None, "{channel}");
    }
    // Nor while the main lane is hidden (`Hide track`), unless Settings `Show hidden tracks` shows it.
    song.channels[1].lanes[0].hidden = true;
    assert_eq!(current(&song, 1, &editor, false), None);
    assert_eq!(current(&song, 1, &editor, true), Some(key(1, 0, 1)));
}

#[test]
fn apply_writes_the_clips_slot_and_gives_a_clip_without_one_the_default_slot() {
    let mut song = song();
    // Clip 1 has no sample: a write gives it the default slot first.
    assert!(apply(&mut song, key(1, 0, 1), Ctl::Rev, 1.0));
    let slot = song.channels[1].lanes[0].clips[0].file.sample.as_deref();
    let mut want = SamplerSlot::default();
    want.file.reverse = true;
    assert_eq!(slot, Some(&want));
    // The same value again changes nothing.
    assert!(!apply(&mut song, key(1, 0, 1), Ctl::Rev, 1.0));
    assert!(apply(&mut song, key(1, 0, 1), Ctl::Level, 0.5));
    assert_eq!(
        song.channels[1].lanes[0].clips[0]
            .file
            .sample
            .as_ref()
            .unwrap()
            .params[0],
        0.5
    );
    // A clip that is gone: nothing.
    let before = song.clone();
    assert!(!apply(&mut song, key(1, 0, 5), Ctl::Level, 0.1));
    assert!(!apply(&mut song, key(4, 0, 1), Ctl::Level, 0.1));
    assert_eq!(song, before);
}

#[test]
fn pitch_plus_12_survives_a_save_and_a_load() {
    let mut song = song();
    assert!(apply(&mut song, key(1, 0, 2), Ctl::Pitch, 0.75));
    let bytes = crate::flm::write(&song).expect("writes");
    let read = crate::flm::read(&bytes).expect("reads");
    let slot = read.channels[1].lanes[0].clips[1].file.sample.as_deref();
    assert_eq!(slot.map(|s| s.file.pitch_semitones), Some(12.0));
    let row = Row::of(slot);
    assert_eq!(row.pitch, 0.75);
    assert_eq!(pitch_text(row.pitch), "Pitch: +12 semitones");
}

/// The keys area of the 1170 × 540 screen at U = 32 under the default panel (y 436 … 540).
fn keyboard() -> Rect {
    crate::ui::main_screen::Regions::new(SCREEN, &crate::ui::offscreen::metrics()).keyboard
}

#[test]
fn the_row_is_30_by_2_u_centred_with_2_u_cells_and_four_equal_sliders_at_ppp_1_and_2() {
    for ppp in [1.0, 2.0] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        assert_eq!(m.u(1.0), U, "ppp {ppp}");
        let rects = layout(keyboard(), &m);
        // 960 × 64 points centred in the area 0.35 U under the keys' top (y 447.2 … 540): x 105 … 1065, y 461.6 …
        // 525.6, the top on a whole device pixel.
        let centre = (keyboard().top() + 0.35 * U + 540.0) / 2.0;
        let top = m.round_px(centre - U);
        assert_eq!(top, if ppp == 1.0 { 462.0 } else { 461.5 });
        let widths: Vec<f32> = rects.iter().map(|r| r.width()).collect();
        assert_eq!(
            widths,
            [
                2.0 * U,
                6.0 * U,
                6.0 * U,
                6.0 * U,
                6.0 * U,
                2.0 * U,
                2.0 * U
            ],
            "ppp {ppp}"
        );
        assert_eq!(rects[0].min, pos2(105.0, top), "ppp {ppp}");
        assert_eq!(rects[6].max, pos2(1065.0, top + 2.0 * U), "ppp {ppp}");
        // Side by side, one height.
        for pair in rects.windows(2) {
            assert_eq!(pair[0].right(), pair[1].left());
            assert_eq!(pair[0].y_range(), pair[1].y_range());
        }
    }
}

#[test]
fn under_2_u_the_row_sits_at_the_top_of_its_area_and_overflows_below() {
    // [flex, 2 U, flex] collapses to the row at the top.
    for ppp in [1.0, 2.0] {
        let m = Metrics::from_pixels_per_point(ppp, DEFAULT_UI_SCALE);
        // Keys areas 2.35 U tall (the row just fits: centred is the top), 1.25 U (the strip open on a 6 U panel), and
        // −0.75 U (a 4 U panel: below the panel's bottom).
        for k in [2.35, 1.25, -0.75] {
            let keys = Rect::from_min_size(pos2(0.0, 400.0), vec2(1170.0, k * U));
            let rects = layout(keys, &m);
            let top = m.round_px(400.0 + 0.35 * U);
            for r in rects {
                assert_eq!((r.top(), r.height()), (top, 2.0 * U), "{k} U, ppp {ppp}");
            }
            assert_eq!(rects[0].left(), 105.0, "still centred across: {k} U");
        }
        // Taller, it is centred in the area.
        let keys = Rect::from_min_size(pos2(0.0, 400.0), vec2(1170.0, 4.35 * U));
        let centre = 400.0 + 0.35 * U + 2.0 * U;
        assert_eq!(layout(keys, &m)[0].top(), m.round_px(centre - U));
    }
}

#[test]
fn a_keys_area_0_tall_hides_the_row() {
    let mut off = Offscreen::new();
    let flat = Rect::from_x_y_ranges(SCREEN.x_range(), 540.0..=540.0);
    let nothing = off.frame(vec![], &mut |_, _| {}).len();
    let mut drawn = None;
    let shapes = off.frame(vec![], &mut |ui, cx| {
        drawn = Some(show(ui, cx, flat, INK, Row::of(None)));
    });
    assert_eq!(drawn, Some(None));
    assert_eq!(shapes.len(), nothing);
    // 1 px tall, it draws (below the panel, as any overflow).
    let shapes = off.frame(vec![], &mut |ui, cx| {
        show(ui, cx, flat.with_min_y(539.0), INK, Row::of(None));
    });
    assert!(shapes.len() > nothing);
}

#[test]
fn a_narrow_keyboard_area_narrows_the_sliders_on_whole_pixels() {
    let m = Metrics::from_pixels_per_point(2.0, DEFAULT_UI_SCALE);
    let area = Rect::from_x_y_ranges(10.0..=(10.0 + 20.3 * U), 0.0..=(4.0 * U));
    let rects = layout(area, &m);
    // The row is the area's width (its right edge on a whole device pixel); the cells keep 2 U, and the sliders share
    // 14.3 U.
    assert_eq!(rects[0].left(), 10.0);
    assert_eq!(rects[6].right(), m.round_px(area.right()));
    assert_eq!(rects[0].width(), 2.0 * U);
    assert_eq!((rects[5].width(), rects[6].width()), (2.0 * U, 2.0 * U));
    for r in &rects[1..5] {
        assert!((r.width() - 14.3 * U / 4.0).abs() <= 0.5, "{r:?}");
        assert_eq!((r.left() * 2.0).fract(), 0.0, "{r:?}");
    }
}

/// The keys area of [`SCREEN`].
fn area() -> Rect {
    keyboard()
}

const INK: Color32 = Color32::from_rgb(200, 80, 40);

/// The row as the app keeps it: each `Set` written into a slot and read back, and the actions in order.
struct Host {
    row: Row,
    actions: Vec<RowAction>,
}

impl Host {
    fn new() -> Self {
        Self {
            row: Row::of(None),
            actions: Vec::new(),
        }
    }

    /// One frame's drawing over [`area`].
    fn draw(&mut self) -> impl FnMut(&mut Ui, &UiCtx) + '_ {
        |ui, cx| {
            let Some(action) = show(ui, cx, area(), INK, self.row) else {
                return;
            };
            if let RowAction::Set { ctl, v, .. } = action {
                let mut slot = SamplerSlot::default();
                let row = self.row;
                // The row's values back into a slot, the new one in, and read again.
                slot.params[..3].copy_from_slice(&[row.level, row.pan, row.speed]);
                set(&mut slot, Ctl::Pitch, row.pitch);
                slot.file.reverse = row.reverse;
                slot.file.stretch_mode = row.sync.into();
                set(&mut slot, ctl, v);
                self.row = Row::of(Some(&slot));
            }
            self.actions.push(action);
        }
    }
}

fn center(ctl: Ctl) -> Pos2 {
    let i = CTLS.iter().position(|&c| c == ctl).unwrap();
    layout(area(), &crate::ui::offscreen::metrics())[i].center()
}

/// A quarter of slider `ctl`'s range: its thumb follows the pointer along the track, 1 U shorter than the control.
fn quarter(ctl: Ctl) -> Vec2 {
    let i = CTLS.iter().position(|&c| c == ctl).unwrap();
    let rect = layout(area(), &crate::ui::offscreen::metrics())[i];
    vec2((rect.width() - U) / 4.0, 0.0)
}

#[test]
fn the_icon_picks_and_the_toggles_flip_as_finished_edits() {
    let (mut off, mut host) = (Offscreen::new(), Host::new());
    let set = |ctl, v, done| RowAction::Set { ctl, v, done };
    for (ctl, want) in [
        (Ctl::Pick, RowAction::Pick),
        (Ctl::Rev, set(Ctl::Rev, 1.0, true)),
        (Ctl::Sync, set(Ctl::Sync, 1.0, true)),
        (Ctl::Rev, set(Ctl::Rev, 0.0, true)),
    ] {
        host.actions.clear();
        off.click(center(ctl), &mut host.draw());
        assert_eq!(host.actions, [want], "{ctl:?}");
    }
    assert!(!host.row.reverse && host.row.sync);
}

#[test]
fn a_slider_drag_is_ongoing_until_its_release_finishes_it() {
    let (mut off, mut host) = (Offscreen::new(), Host::new());
    // An eighth of the track left is −0.125, a quarter −0.25.
    let steps = [-quarter(Ctl::Pan) / 2.0, -quarter(Ctl::Pan)];
    off.drag(center(Ctl::Pan), &steps, &mut host.draw());
    let set = |v, done| RowAction::Set {
        ctl: Ctl::Pan,
        v,
        done,
    };
    assert_eq!(
        host.actions,
        [set(0.375, false), set(0.25, false), set(0.25, true)]
    );
    // Pitch moves in whole semitones: a quarter right is +12.
    host.actions.clear();
    off.drag(center(Ctl::Pitch), &[quarter(Ctl::Pitch)], &mut host.draw());
    let last = host.actions.last().copied();
    let want = RowAction::Set {
        ctl: Ctl::Pitch,
        v: 0.75,
        done: true,
    };
    assert_eq!(last, Some(want));
    assert_eq!(pitch_text(host.row.pitch), "Pitch: +12 semitones");
}

#[test]
fn the_row_shows_each_text_law_in_the_channel_colour_and_the_toggles_lit_when_on() {
    let (mut off, mut host) = (Offscreen::new(), Host::new());
    host.row.reverse = true;
    let shapes = off.frame(vec![], &mut host.draw());
    let texts = Offscreen::texts(&shapes);
    let text = crate::theme::theme_by_id(0).colors.text.to_color32();
    let want = [
        ("Level: 0.0 dB", INK),
        ("Pan: center", INK),
        ("Speed: 1.0x", INK),
        ("Pitch: 0 semitones", INK),
        ("REV", text),
        ("SYNC", text.gamma_multiply(0.25)),
    ];
    for (label, color) in want {
        assert!(
            texts.contains(&(label.to_owned(), color)),
            "{label}: {texts:?}"
        );
    }
}
