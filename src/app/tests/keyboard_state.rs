//! The keyboard's state per channel: the keys the engine sounds at the current channel's input light at T = 0.7, and
//! each channel's piano scroll and zoom come back when it is current again.

use super::editor_wiring::EDITOR_TOP;
use super::*;
use crate::song::Event;
use crate::ui::keyboard::KeyView;
use eframe::egui::Rect;
use honeydew_audio::device::AudioRenderer;

#[test]
fn the_keyboard_starts_with_middle_c_centered_and_keeps_manual_scroll() {
    for (width, scale, wheels) in [(480.0, 0, false), (1170.0, 1, true), (2048.0, 2, false)] {
        let mut rig = Rig::new();
        rig.app.main_screen = MainScreen::default();
        rig.app.settings.ui_scale_index = scale;
        rig.app.settings.show_wheels = wheels;
        rig.app
            .add_bare_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
        let draw = |rig: &mut Rig| {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 540.0))),
                ..Default::default()
            };
            rig.out = rig.ctx.run(raw, |ctx| rig.app.frame(ctx, |_, _| None));
        };
        draw(&mut rig);
        // Outside a frame, read_response returns the preceding pass's geometry.
        draw(&mut rig);
        let rect = rig
            .ctx
            .read_response(egui::Id::new("keyboard"))
            .unwrap()
            .rect;
        let m = Metrics::new(&rig.ctx, scale);
        let keys = crate::ui::keyboard::keyboard_keys(
            rect.width(),
            m.u(1.0),
            rig.app.main_screen.keys_view,
            0..=119,
            None,
        );
        let middle_c = keys.iter().find(|k| k.midi == 60).unwrap().rect(rect);
        assert!(
            (middle_c.center().x - rect.center().x).abs() < 0.01,
            "width {width}, scale {scale}, wheels {wheels}: rect {rect:?}, key {middle_c:?}, view {:?}, U {}",
            rig.app.main_screen.keys_view,
            m.u(1.0),
        );

        rig.app.main_screen.keys_view.left -= 3.0;
        let scrolled = rig.app.main_screen.keys_view;
        draw(&mut rig);
        assert_eq!(rig.app.main_screen.keys_view, scrolled);
    }
}

/// Two MiniSynth channels (1 and 2), the audio started, channel 1 selected: channel 1 plays MIDI 60 and channel 2
/// MIDI 67, each for two beats from beat 0 (120 BPM).
fn two_synths() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    for (channel, key) in [(1, 60), (2, 67)] {
        let lane = &mut rig.app.song.channels[channel].lanes[0];
        lane.add_clip(0, 4.0).events.push(Event::Note {
            pos: 0,
            len_beats: 2.0,
            key,
            velocity: Event::DEFAULT_VELOCITY,
            pan: Event::DEFAULT_PAN,
            slide: false,
        });
    }
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig
}

impl Rig {
    /// The engine the app sent last renders `beats` beats at 120 BPM and 48 kHz; then a frame polls it.
    fn render_beats(&mut self, beats: f64) {
        {
            let mut rec = self.rec.0.borrow_mut();
            let engine = rec.engine.as_mut().expect("an engine");
            let frames = (beats * 24_000.0) as usize;
            engine.render(&mut vec![0.0; frames * 2], 2);
        }
        self.frame(vec![]);
    }

    /// A tap on the play button (Play, or Stop while playing), and a frame that polls the engine after it.
    fn tap_play(&mut self) {
        self.click(PLAY);
        self.frame(vec![]);
    }
}

#[test]
fn song_playback_lights_the_current_channels_keys_until_their_note_off() {
    let mut rig = two_synths();
    assert_eq!(rig.app.key_input().sounding, 0);
    rig.tap_play();
    rig.render_beats(1.0);
    assert_eq!(rig.app.key_input().sounding, 1 << 60);
    assert_eq!(rig.app.key_input().notes, 0, "no live key: T = 0.7");
    // Another channel's notes light its own keys only.
    rig.app.selected_channel = 2;
    assert_eq!(rig.app.key_input().sounding, 1 << 67);
    rig.app.selected_channel = 1;
    // Both notes end at beat 2.
    rig.render_beats(1.5);
    assert_eq!(rig.app.key_input().sounding, 0);
}

#[test]
fn stop_clears_every_key_the_song_sounds() {
    let mut rig = two_synths();
    rig.tap_play();
    rig.render_beats(1.0);
    assert_eq!(rig.app.key_input().sounding, 1 << 60);
    rig.tap_play();
    assert_eq!(rig.app.key_input().sounding, 0);
}

/// A tap on the roll's key column previews its key: it lights at T = 0.7 (no live key holds it) until it lifts.
#[test]
fn a_piano_roll_preview_lights_its_key_until_it_lifts() {
    let mut rig = Rig::with_synth();
    // An engine the recorder plays: the next build's.
    rig.app.song_edited(SongEdit::Final, 0);
    rig.edit(1);
    rig.slide_split();
    let keys = Rect::from_min_max(pos2(0.0, EDITOR_TOP + 1.5 * U), pos2(1.5 * U, PANEL_TOP));
    rig.take();
    rig.press(keys.center());
    let sent = rig.take();
    let key: u8 = (sent.first())
        .and_then(|c| c.strip_prefix("NoteOn 1 "))
        .and_then(|c| c.split(' ').next()?.parse().ok())
        .unwrap_or_else(|| panic!("a preview: {sent:?}"));
    rig.frame(vec![]);
    assert_eq!(rig.app.key_input().sounding, 1 << key);
    assert_eq!(rig.app.key_input().notes, 0);
    rig.release(keys.center());
    rig.frame(vec![]);
    assert_eq!(rig.app.key_input().sounding, 0);
}

#[test]
fn each_channel_gets_its_keyboard_scroll_and_zoom_back() {
    let mut rig = two_synths();
    let a = KeyView {
        left: 40.0,
        white_w: 3.0,
    };
    let b = KeyView {
        left: 30.0,
        white_w: 2.0,
    };
    rig.app.main_screen.keys_view = a;
    rig.frame(vec![]);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(
        rig.app.main_screen.keys_view, a,
        "none stored for channel 2: the view stays"
    );
    rig.app.main_screen.keys_view = b;
    rig.frame(vec![]);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, a);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, b);
}

/// The roll tints the rows whose key sounds on its channel, the engine's keys included: song playback tints the row
/// of the note it plays.
#[test]
fn song_playback_tints_the_rolls_row_of_the_key_it_sounds() {
    let mut rig = two_synths();
    rig.edit(1);
    rig.slide_split();
    rig.tap_play();
    rig.render_beats(1.0);
    assert_eq!(rig.app.slots.piano_roll.sounding, 1 << 60);
    let lit = crate::theme::channel_color(rig.app.song.channels[1].hue, &rig.app.theme)
        .to_color32()
        .gamma_multiply(0.1);
    let grid = rig
        .ctx
        .read_response(egui::Id::new("piano_roll").with("grid"));
    let grid = grid.expect("the roll's grid").rect;
    let rows = rig.rects(lit);
    let rows: Vec<_> = (rows.iter())
        .filter(|r| r.intersects(grid) && r.height() < U)
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].x_range(), grid.x_range());
}

/// A new song (`SongEdit::Replaced`) keeps no channel's keyboard view: its channels' ids are its own, so the old
/// song's views would land on whichever channel shares an id. The view shown stays as it stands.
#[test]
fn a_loaded_song_starts_with_no_keyboard_views_kept() {
    let mut rig = two_synths();
    let a = KeyView {
        left: 40.0,
        white_w: 3.0,
    };
    let b = KeyView {
        left: 30.0,
        white_w: 2.0,
    };
    rig.app.main_screen.keys_view = a;
    rig.frame(vec![]);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    rig.app.main_screen.keys_view = b;
    rig.frame(vec![]);
    // The load selects channel 1, whose id the old song's channel 1 had.
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.selected_channel, 1);
    assert_eq!(rig.app.main_screen.keys_view, b);
    // Nor does channel 2 get the view the old one left.
    rig.app.main_screen.keys_view = a;
    rig.frame(vec![]);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, a);
}

/// The playhead knob on the playlist's ruler at `beat` (`transport.rs`'s `ruler`): tick 0 at the grid's left edge, a
/// beat 1.2 U at the default zoom.
fn ruler(beat: f32) -> Pos2 {
    pos2(1.5 * U + 2.0 + beat * 1.2 * U, 0.75 * U)
}

/// Position jumps through the app: a seek while playing re-syncs the engine.
/// A key the song holds that a note covers at the new beat stays lit at 0.2; one no
/// note covers there goes dark; a covered key that was not sounding stays dark.
#[test]
fn a_playhead_drag_while_playing_keeps_a_covered_key_lit_and_clears_an_uncovered_one() {
    // From the song start, beat 1 played, the knob dropped at `to` beats: it is held from before Play (playing, it
    // fades out and takes no new press), and Play is the keymap's Space.
    let seek = |to: f32| {
        let mut rig = two_synths();
        // Channel 1: MIDI 60 for beats 0 to 1.5 and 2.5 to 3.5; MIDI 64 for beats 2.25 to 3.5.
        let clip = &mut rig.app.song.channels[1].lanes[0].clips[0];
        if let Event::Note { len_beats, .. } = &mut clip.events[0] {
            *len_beats = 1.5;
        }
        for (beat, key) in [(2.5, 60), (2.25, 64)] {
            clip.events.push(Event::Note {
                pos: (beat * 128.0) as u32,
                len_beats: 3.5 - beat,
                key,
                velocity: Event::DEFAULT_VELOCITY,
                pan: Event::DEFAULT_PAN,
                slide: false,
            });
        }
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
        let (from, to) = (ruler(0.0), ruler(to));
        rig.press(from);
        rig.type_key(egui::Key::Space, true);
        rig.type_key(egui::Key::Space, false);
        rig.render_beats(1.0);
        assert_eq!(rig.app.key_input().sounding, 1 << 60);
        rig.take();
        rig.rec.0.borrow_mut().tick = 128;
        rig.drag_to(from + (to - from) / 2.0);
        rig.drag_to(to);
        rig.release(to);
        rig.frame(vec![]);
        assert!(rig.app.main_screen.playing);
        rig
    };
    // Into the second 60, which covers it, and past 64's start: 60 stays lit, 64 never lights.
    let mut rig = seek(3.0);
    assert!(rig.take().contains(&"Seek 384".to_string()));
    assert_eq!(rig.app.key_input().sounding, 1 << 60);
    // Into the gap between them (beat 2): nothing covers 60.
    let mut rig = seek(2.2);
    assert!(rig.take().contains(&"Seek 256".to_string()));
    assert_eq!(rig.app.key_input().sounding, 0);
}

/// Clip mute (CLHd@36) through the app: muting a clip while playing darkens the keys only
/// it held; unmuting it lights none, since no note starts mid-span.
#[test]
fn a_clip_mute_while_playing_darkens_its_keys_and_an_unmute_lights_none() {
    let mut rig = two_synths();
    rig.tap_play();
    rig.render_beats(1.0);
    assert_eq!(rig.app.key_input().sounding, 1 << 60);
    for muted in [true, false] {
        rig.app.song.channels[1].lanes[0].clips[0].muted = muted;
        rig.app.song_edited(SongEdit::Final, 128);
        rig.frame(vec![]);
        let log = rig.take();
        assert!(
            log.contains(&"ReplaceSong adopting".to_string()),
            "a Mute edit keeps the playing engine: {log:?}"
        );
        assert_eq!(rig.app.key_input().sounding, 0, "muted {muted}");
    }
    // Channel 2's 67, which no edit touched, sounds on.
    rig.app.selected_channel = 2;
    assert_eq!(rig.app.key_input().sounding, 1 << 67);
}

/// The views are keyed by channel id: each follows its channel across an index shift (a delete) and its undo.
#[test]
fn keyboard_views_follow_their_channel_across_a_delete_and_its_undo() {
    let mut rig = two_synths();
    let a = KeyView {
        left: 40.0,
        white_w: 3.0,
    };
    let b = KeyView {
        left: 30.0,
        white_w: 2.0,
    };
    rig.app.main_screen.keys_view = a;
    rig.frame(vec![]);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    rig.app.main_screen.keys_view = b;
    rig.frame(vec![]);
    // Channel 2 moves to index 1.
    rig.app.delete_channel(1, 0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, b);
    // The undo puts channel 1 back at index 1.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, crate::history::Step::Undo, 0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, a);
}

/// Undo restores the channel id counter, so a channel added after an undo took one away can get that channel's
/// id. It opens with the view as it stands, never the one the gone channel left.
#[test]
fn a_new_channel_on_an_id_an_undo_freed_gets_no_stale_keyboard_view() {
    let mut rig = Rig::with_synth();
    let a = KeyView {
        left: 40.0,
        white_w: 3.0,
    };
    let b = KeyView {
        left: 30.0,
        white_w: 2.0,
    };
    rig.app.main_screen.keys_view = a;
    rig.frame(vec![]);
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    let id = rig.app.song.channels[2].id;
    rig.frame(vec![]);
    rig.app.main_screen.keys_view = b;
    rig.frame(vec![]);
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, crate::history::Step::Undo, 0);
    rig.frame(vec![]);
    assert_eq!(
        (rig.app.selected_channel, rig.app.main_screen.keys_view),
        (1, a)
    );
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    assert_eq!(rig.app.song.channels[2].id, id, "the id comes back");
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.keys_view, a);
}

/// The roll's rows tint for the channel it shows in the frame the bottom editor re-targets to another channel's
/// clip, not for the one it showed as the frame began.
#[test]
fn the_rolls_rows_tint_for_its_new_channel_in_the_frame_it_retargets() {
    let mut rig = two_synths();
    rig.edit(1);
    rig.slide_split();
    // Z on channel 1: its 48 sounds there.
    rig.frame(vec![super::typed::typed(
        egui::Key::Z,
        true,
        egui::Modifiers::NONE,
    )]);
    rig.frame(vec![]);
    assert_eq!(rig.app.slots.piano_roll.sounding, 1 << 48);
    // Channel 2 current: the editor follows its current clip within the frame.
    let lane = &mut rig.app.song.channels[2].lanes[0];
    lane.file.current_clip_id = lane.clips[0].id;
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    assert_eq!(rig.app.slots.piano_roll.sounding, 0);
}
