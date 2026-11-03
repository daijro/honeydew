//! The transport and timeline gestures: Play starts
//! from the song position and remembers it as the play-start point, and stops where it is; REV stops and goes back to
//! the play-start point, or from there to the song start; every seek moves the play-start point, and one during
//! playback turns recording off. The playlist's playhead knob plays on a tap and seeks where a drag lets go.

use super::*;
use crate::song::PPQ;
use honeydew_audio::device::AudioRenderer;

const BEAT: u64 = PPQ as u64;
/// REC, two slots left of play.
const REC: Pos2 = pos2(585.0 - 2.0 * 1.75 * U, BAR_Y);

/// On the playlist's ruler at `beat`: tick 0 is at the grid's left edge (1.5 U + t, t = 2 points), a beat is 1.2 U at
/// the default zoom.
fn ruler(beat: f32) -> Pos2 {
    pos2(1.5 * U + 2.0 + beat * 1.2 * U, 0.75 * U)
}

/// The playhead knob (the play box, its left edge on the playhead) on the ruler's middle line, on the transport at
/// `tick`.
fn knob(tick: u64) -> Pos2 {
    ruler(tick as f32 / BEAT as f32)
}

impl Rig {
    /// The playhead x the play box on the ruler shows as last drawn (its centre less half its width), to a thousandth
    /// of a point ([`r3`]).
    fn box_x(&self) -> Option<f32> {
        self.play_box_in(0.0..RULER).map(|b| b.0)
    }

    /// Lets the play box fade out (or in): at alpha 0 it takes no touch.
    fn fade(&mut self) {
        // A fifth of the way a frame: gone within 31 frames.
        for _ in 0..40 {
            self.frame(vec![]);
        }
    }

    /// The output reports the transport at `tick`.
    fn at(&mut self, tick: u64) {
        self.rec.0.borrow_mut().tick = tick;
    }

    /// The commands sent since the last call, but the metronome's.
    fn transport(&mut self) -> Vec<String> {
        let mut log = self.take();
        log.retain(|c| !c.starts_with("Metronome"));
        log
    }

    fn playing(&self) -> bool {
        self.app.main_screen.playing
    }
}

/// [`Rig::with_synth`] with a 16-beat clip (the song ends at beat 16), and no count-in.
fn song_rig() -> Rig {
    let mut rig = Rig::with_synth();
    rig.app.settings.count_in = false;
    rig.app.song.channels[1].lanes[0].add_clip(0, 16.0);
    rig.frame(vec![]);
    rig.take();
    rig
}

#[test]
fn play_starts_at_the_song_position_and_a_second_tap_stops_there_with_recording_off() {
    let mut rig = song_rig();
    rig.at(3 * BEAT);
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.playing());
    assert_eq!(rig.app.main_screen.play_start, 3 * BEAT);
    rig.at(7 * BEAT);
    rig.click(PLAY);
    assert!(!rig.playing());
    assert!(!rig.app.main_screen.armed, "recording stops");
    // No seek: it stops where it is (the engine's Stop releases every voice and restores the params,
    // `tests/engine_transport.rs`).
    assert_eq!(rig.transport(), ["Play", "Stop"]);
    assert_eq!(rig.app.main_screen.play_start, 3 * BEAT);
}

#[test]
fn rev_stops_and_goes_back_to_the_play_start_point_and_from_there_to_the_song_start() {
    let mut rig = song_rig();
    rig.at(4 * BEAT);
    rig.click(PLAY);
    rig.at(9 * BEAT);
    rig.click(REV);
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Play", "Stop", "Seek 512"]);
    // The transport is there now: the next REV goes to the song start, which becomes the play-start point, and resets
    // the sound.
    rig.at(4 * BEAT);
    rig.click(REV);
    assert_eq!(rig.transport(), ["Seek 0", "ResetSound"]);
    assert_eq!(rig.app.main_screen.play_start, 0);
    rig.at(0);
    rig.click(REV);
    assert_eq!(rig.transport(), ["Seek 0", "ResetSound"]);
    // Armed and playing: REV stops the same way, so the recording ends too.
    rig.click(REC);
    rig.click(PLAY);
    rig.click(REV);
    assert!(!rig.app.main_screen.armed);
    assert_eq!(rig.transport(), ["Play", "Stop", "Seek 0", "ResetSound"]);
}

/// REV at the play-start point resets the sound: the engine silences every voice at once, a held keyboard note
/// included; a REV elsewhere only stops and goes back.
#[test]
fn rev_at_the_play_start_point_silences_the_engine() {
    let mut rig = song_rig();
    // The output gets an engine the rig can render.
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    rig.take();
    rig.app.audio.send(AudioCommand::NoteOn {
        channel: 1,
        key: 69,
        velocity: 1.0,
    });
    let energy = |rig: &mut Rig| {
        let mut output = vec![0.0; 1024 * 2];
        let mut rec = rig.rec.0.borrow_mut();
        rec.engine
            .as_mut()
            .expect("an engine")
            .render(&mut output, 2);
        output.iter().map(|s| s * s).sum::<f32>()
    };
    assert!(energy(&mut rig) > 0.01);
    rig.take();
    rig.at(BEAT);
    rig.click(REV);
    assert_eq!(rig.transport(), ["Seek 0"]);
    assert!(energy(&mut rig) > 0.01);
    rig.at(0);
    rig.click(REV);
    assert_eq!(rig.transport(), ["Seek 0", "ResetSound"]);
    assert_eq!(energy(&mut rig), 0.0);
}

#[test]
fn a_ruler_tap_while_stopped_seeks_and_moves_the_play_start_point_and_one_while_playing_stops() {
    let mut rig = song_rig();
    rig.click(ruler(3.0));
    assert_eq!(rig.transport(), ["Seek 384"]);
    assert_eq!(rig.app.main_screen.play_start, 3 * BEAT);
    // REV from elsewhere comes back there.
    rig.at(10 * BEAT);
    rig.click(REV);
    assert_eq!(rig.transport(), ["Seek 384"]);
    // Playing, a ruler tap stops instead and goes back to the play-start point.
    rig.at(3 * BEAT);
    rig.click(PLAY);
    rig.at(5 * BEAT);
    rig.click(ruler(9.0));
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Play", "Stop", "Seek 384"]);
    assert_eq!(rig.app.main_screen.play_start, 3 * BEAT);
}

/// A ruler tap that stops returns to the play-start point, as REV's first press does; at
/// the play-start point it stays there (no REV second press: no song start, no sound reset).
#[test]
fn a_ruler_tap_that_stops_goes_back_to_where_play_started() {
    let mut rig = song_rig();
    rig.click(ruler(4.0));
    rig.at(4 * BEAT);
    rig.click(PLAY);
    rig.take();
    // It runs on to beat 9 and past.
    for tick in [5 * BEAT, 7 * BEAT, 9 * BEAT + 40] {
        rig.at(tick);
        rig.frame(vec![]);
    }
    rig.click(ruler(12.0));
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Stop", "Seek 512"]);
    rig.at(4 * BEAT);
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.play_start, 4 * BEAT);
    assert_eq!(rig.box_x(), Some(r3(knob(4 * BEAT).x)));
    // Stopped on the play-start point itself, it stays there: REV's second press would go to the song start.
    rig.click(PLAY);
    rig.take();
    rig.click(ruler(12.0));
    assert_eq!(rig.transport(), ["Stop", "Seek 512"]);
    // The Play button's stop stays where it is.
    rig.click(PLAY);
    rig.at(6 * BEAT);
    rig.click(PLAY);
    assert_eq!(rig.transport(), ["Play", "Stop"]);
}

/// The playhead line, following: it sits at the song position every drawn frame. The output reports
/// the transport once an audio block (here 2048 frames at 48 kHz, 43 ms: 11 ticks at 120 BPM); between its steps the
/// line moves on at the tempo on every 60 Hz frame, never more than a block ahead, and a wrap shows at once.
#[test]
fn playing_the_playlists_line_moves_on_every_frame_between_the_outputs_steps() {
    let mut rig = song_rig();
    let text = rig.app.theme.colors.text.to_color32();
    let tracks = egui::Rangef::new(RULER, PANEL_TOP);
    let line = |rig: &Rig| {
        (rig.rects(text).iter())
            .find(|r| r.width() < U && r.y_range() == tracks)
            .map(|r| crate::ui::timeline::testing::line_x(*r, 1.0))
    };
    rig.click(PLAY);
    let (t0, block) = (rig.time, 2048.0 / 48_000.0);
    let rate = rig.app.song.tempo_bpm / 60.0 * BEAT as f64;
    let mut drawn = Vec::new();
    for i in 1..=40 {
        let t = f64::from(i) / 60.0;
        let tick = ((t / block).floor() * block * rate) as u64;
        rig.at(tick);
        rig.next_frame_at(t0 + t);
        rig.frame(vec![]);
        let lead = rig.app.audio.playhead_lead();
        assert!(
            lead as f64 <= rate * (block + 1.0 / 60.0),
            "frame {i}: {lead}"
        );
        // The line is drawn there, on the nearest pixel.
        let x = knob(tick + lead).x;
        assert!(
            line(&rig).is_some_and(|l| (l - x).abs() <= 0.5),
            "frame {i}: {x}"
        );
        drawn.push(tick + lead);
    }
    assert!(drawn[8..].windows(2).all(|w| w[1] > w[0]), "{drawn:?}");
    // A loop wrap: back at once.
    rig.at(64);
    rig.next_frame_at(t0 + 41.0 / 60.0);
    rig.frame(vec![]);
    assert!(line(&rig).is_some_and(|l| (l - knob(64).x).abs() <= 0.5));
}

/// The output's transport over 40 frames at 60 Hz, playing from `from` in 43 ms blocks at 120 BPM and wrapping into
/// `wrap` (start, end) at its end: the ticks drawn (the snapshot's plus the lead).
fn play_blocks(rig: &mut Rig, from: u64, wrap: Option<(u64, u64)>) -> Vec<u64> {
    rig.at(from);
    rig.frame(vec![]);
    rig.click(PLAY);
    let (t0, block) = (rig.time, 2048.0 / 48_000.0);
    let drawn = (1..=40)
        .map(|i| {
            let t = f64::from(i) / 60.0;
            let mut tick = from + (t / block).floor() as u64 * 2048 * 256 / 48_000;
            if let Some((start, end)) = wrap.filter(|&(_, end)| tick >= end) {
                tick = start + (tick - start) % (end - start);
            }
            rig.at(tick);
            rig.next_frame_at(t0 + t);
            rig.frame(vec![]);
            tick + rig.app.audio.playhead_lead()
        })
        .collect();
    rig.click(PLAY);
    drawn
}

/// The line never runs past the tick the engine wraps at, waiting for the snapshot that shows the wrap:
/// the play loop's end, else the song length (whole bars, here 16 beats). Armed, without a loop, the recording plays on
/// past the song's end, and the line with it.
#[test]
fn the_playlists_line_never_runs_past_where_the_engine_wraps() {
    let mut rig = song_rig();
    // From 1990, the last block before the end ends at 2044, 4 ticks short of it.
    let song_end = play_blocks(&mut rig, 1990, Some((0, 16 * BEAT)));
    assert!(song_end.iter().all(|&t| t <= 16 * BEAT), "{song_end:?}");
    rig.app.song.play_loop = Some((BEAT as u32, 5 * BEAT as u32));
    rig.frame(vec![]);
    let looped = play_blocks(&mut rig, 582, Some((BEAT, 5 * BEAT)));
    assert!(looped.iter().all(|&t| t <= 5 * BEAT), "{looped:?}");
    assert!(
        looped.iter().any(|&t| t < 2 * BEAT),
        "it wrapped: {looped:?}"
    );
    rig.app.song.play_loop = None;
    rig.click(REC);
    assert!(
        play_blocks(&mut rig, 15 * BEAT, None)
            .iter()
            .any(|&t| t > 16 * BEAT)
    );
}

/// Follow playhead keeps the drawn playhead, lead included, a quarter of the grid in, so the line stands
/// still while the output steps by blocks under it.
#[test]
fn following_the_playlists_line_stands_still_a_quarter_in() {
    let mut rig = song_rig();
    rig.app.song.channels[1].lanes[0].add_clip(2048, 48.0);
    rig.app.settings.continuous_scroll = true;
    let text = rig.app.theme.colors.text.to_color32();
    let tracks = egui::Rangef::new(RULER, PANEL_TOP);
    rig.at(20 * BEAT);
    rig.frame(vec![]);
    rig.click(PLAY);
    let (t0, block) = (rig.time, 2048.0 / 48_000.0);
    let (mut xs, mut leads) = (Vec::new(), Vec::new());
    for i in 1..=30 {
        let t = f64::from(i) / 60.0;
        rig.at(20 * BEAT + (t / block).floor() as u64 * 2048 * 256 / 48_000);
        rig.next_frame_at(t0 + t);
        rig.frame(vec![]);
        leads.push(rig.app.audio.playhead_lead());
        let rects = rig.rects(text);
        let line = (rects.iter()).find(|r| r.width() < U && r.y_range() == tracks);
        xs.push(line.expect("the line").center().x);
    }
    assert!(leads.iter().any(|&l| l > 0), "{leads:?}");
    assert!(xs[1..].iter().all(|&x| (x - xs[1]).abs() <= 1.0), "{xs:?}");
}

#[test]
fn a_tap_on_the_playhead_knob_plays() {
    let mut rig = song_rig();
    rig.at(2 * BEAT);
    rig.click(knob(2 * BEAT));
    assert!(rig.playing());
    assert_eq!(rig.transport(), ["Play"]);
    assert_eq!(rig.app.main_screen.play_start, 2 * BEAT);
    // Playing, the knob fades out and, faded, takes no tap, as the drum editor's (`timeline::knob_shown`): the tap is
    // the ruler's, which stops and goes back to the play-start point.
    rig.fade();
    rig.click(knob(2 * BEAT));
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Stop", "Seek 256"]);
}

#[test]
fn a_playhead_drag_seeks_to_the_beat_it_lets_go_on_within_the_song() {
    let mut rig = song_rig();
    let drag = |rig: &mut Rig, from: Pos2, to: Pos2| {
        rig.press(from);
        rig.drag_to(from + (to - from) / 2.0);
        rig.drag_to(to);
        rig.release(to);
    };
    // Stopped: onto the beat grid, and the play-start point moves.
    drag(&mut rig, knob(0), ruler(5.3));
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Seek 640"]);
    assert_eq!(rig.app.main_screen.play_start, 5 * BEAT);
    // Past the song end: its end.
    rig.at(5 * BEAT);
    drag(&mut rig, knob(5 * BEAT), ruler(25.0));
    assert_eq!(rig.transport(), ["Seek 2048"]);
    // Playing, the knob fades out and, faded, takes no drag, as the drum editor's (`timeline::knob_shown`): no seek,
    // and it plays on.
    rig.at(6 * BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.fade();
    rig.take();
    drag(&mut rig, knob(6 * BEAT), ruler(2.2));
    assert!(rig.playing());
    assert!(rig.app.main_screen.armed);
    assert_eq!(rig.transport(), Vec::<String>::new());
    assert_eq!(rig.app.main_screen.play_start, 6 * BEAT);
}

/// Regression: the piano roll's handle did not drag. Every editor's play box is the playhead knob: stopped, a drag seeks to
/// The snapped beat it lets go on.
#[test]
fn the_piano_rolls_playhead_knob_drags_and_seeks_where_it_lets_go() {
    let mut rig = song_rig();
    rig.edit(1);
    rig.slide_split();
    rig.take();
    // The roll's ruler: a beat is 64 points from the grid's left edge (1.5 U + t, t = 2 points). The box at beat 0,
    // pressed on the playhead and dragged 3 U right: beat 1.5.
    let beat = |b: f32| pos2(1.5 * U + 2.0 + b * 64.0, 0.75 * U);
    let to = beat(1.5);
    rig.press(beat(0.0));
    rig.drag_to(beat(0.75));
    rig.drag_to(to);
    // The line and the box follow the finger.
    assert_eq!(rig.box_x(), Some(r3(to.x)));
    rig.release(to);
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["Seek 192"]);
    // Until the output reports the seek, the box stays on the drop.
    rig.frame(vec![]);
    assert_eq!(rig.box_x(), Some(r3(to.x)));
    // Held across a Play (Space), the drop seeks and it plays on, as the playlist's knob.
    rig.at(192);
    rig.press(to);
    rig.type_key(egui::Key::Space, true);
    rig.type_key(egui::Key::Space, false);
    assert!(rig.playing());
    rig.drag_to(beat(2.25));
    rig.drag_to(beat(3.0));
    rig.release(beat(3.0));
    assert!(rig.playing());
    assert_eq!(rig.transport(), ["Play", "Seek 384"]);
}

#[test]
fn a_dropped_knob_stays_where_it_was_dropped_until_the_transport_gets_there_or_a_quarter_second() {
    // The output reports the seek a frame or more later; meanwhile the box must not flash back.
    let mut rig = song_rig();
    let drop = |rig: &mut Rig, from: Pos2, to: Pos2| {
        rig.press(from);
        rig.drag_to(to);
        rig.release(to);
        rig.take();
        rig.time
    };
    let frame_at = |rig: &mut Rig, t: f64| {
        rig.next_frame_at(t);
        rig.frame(vec![]);
        rig.box_x()
    };
    let t = drop(&mut rig, knob(0), ruler(5.3));
    let at = |p: Pos2| Some(r3(p.x));
    // Still at tick 0 in the output: the box stays on the dropped beat 5.
    assert_eq!(frame_at(&mut rig, t + 0.02), at(ruler(5.0)));
    // The output gets there: from then on it follows the output again.
    rig.at(5 * BEAT + 3);
    assert_eq!(frame_at(&mut rig, t + 0.04), at(knob(5 * BEAT + 3)));
    rig.at(0);
    assert_eq!(frame_at(&mut rig, t + 0.06), at(knob(0)));
    // An output that never gets there: back to it after a quarter second.
    let t = drop(&mut rig, knob(0), ruler(8.2));
    assert_eq!(frame_at(&mut rig, t + 0.1), at(ruler(8.0)));
    assert_eq!(frame_at(&mut rig, t + 0.2), at(ruler(8.0)));
    assert_eq!(frame_at(&mut rig, t + 0.3), at(knob(0)));
}

/// A recorded note: (clip id, clip start, pos, key, length in beats).
type Note = (u32, u32, u32, u8, f64);

/// Channel 1's notes, by lane.
fn recorded(rig: &Rig) -> Vec<Vec<Note>> {
    let lanes = rig.app.song.channels[1].lanes.iter();
    let notes = lanes.map(|l| {
        let clips = l.clips.iter().flat_map(|c| {
            c.events.iter().filter_map(|e| match *e {
                crate::song::Event::Note {
                    pos,
                    len_beats,
                    key,
                    ..
                } => Some((c.id, c.start_tick, pos, key, len_beats)),
                _ => None,
            })
        });
        clips.collect()
    });
    notes.collect()
}

#[test]
fn a_seek_while_recording_notes_ends_the_take_and_a_held_note_keeps_length_0() {
    // A seek clears the open notes without closing them; the UI's seek
    // turns REC off as well, so no new take starts.
    let mut rig = song_rig();
    rig.at(BEAT);
    rig.click(REC);
    // The playhead knob held from before Play (playing, it fades out and takes no new press), Play by the
    // keymap's Space.
    rig.press(knob(BEAT));
    rig.type_key(egui::Key::Space, true);
    rig.type_key(egui::Key::Space, false);
    assert!(rig.playing());
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    // Its clip at beat 0, which the 16-beat clip covers: a new notes lane.
    assert_eq!(recorded(&rig), [vec![], vec![(1, 0, 2 * PPQ, 48, 0.0)]]);
    rig.at(3 * BEAT);
    rig.drag_to(ruler(6.0));
    rig.drag_to(ruler(9.2));
    rig.release(ruler(9.2));
    assert!(rig.playing() && !rig.app.main_screen.armed);
    rig.at(10 * BEAT);
    rig.type_key(egui::Key::Z, false);
    assert_eq!(recorded(&rig), [vec![], vec![(1, 0, 2 * PPQ, 48, 0.0)]]);
    assert_eq!(rig.app.main_screen.takes, []);
}

#[test]
fn an_empty_open_clip_covering_the_takes_start_is_replaced_and_its_editor_shows_the_take() {
    let mut rig = song_rig();
    rig.edit(1);
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(1));
    rig.at(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    // The piano roll's clip (lane 0, id 1, empty, over beat 0) goes; the take's clip (id 2) takes its place, and the
    // piano roll stays open on it while it records (issue #26).
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::PianoRoll(1));
    assert_eq!(recorded(&rig), [vec![(2, 0, 2 * PPQ, 48, 0.0)]]);
    assert_eq!(rig.app.song.channels[1].lanes[0].clips.len(), 1);
}

#[test]
fn a_playhead_drop_on_a_short_song_clamps_to_its_end() {
    // A song whose last clip ends at beat 2 (shorter than a bar): a drop at beat 3.6 seeks to the song end, beat 2
    // (the knob's range is [0, song end]), not to the one bar the playlist's view counts.
    let mut rig = Rig::with_synth();
    rig.app.settings.count_in = false;
    rig.app.song.channels[1].lanes[0].add_clip(0, 2.0);
    rig.frame(vec![]);
    rig.take();
    let (from, to) = (knob(0), ruler(3.6));
    rig.press(from);
    rig.drag_to(from + (to - from) / 2.0);
    rig.drag_to(to);
    rig.release(to);
    assert_eq!(rig.transport(), ["Seek 256"]);
}

#[test]
fn a_playhead_drop_on_an_empty_song_seeks_to_0() {
    // The knob clamps to the song's own end, 0 without clips (the view's one bar is the view's).
    let mut rig = Rig::with_synth();
    rig.app.settings.count_in = false;
    rig.frame(vec![]);
    rig.take();
    let (from, to) = (knob(0), ruler(3.6));
    rig.press(from);
    rig.drag_to(from + (to - from) / 2.0);
    rig.drag_to(to);
    rig.release(to);
    assert_eq!(rig.transport(), ["Seek 0"]);
}

/// Channel 1's Level (instance 0).
const LEVEL: crate::song::AutomationTarget = crate::song::AutomationTarget {
    instance_id: 0,
    param: 0,
};

/// The undo steps there are (undone, then redone back).
fn steps(rig: &mut Rig) -> usize {
    let history = &mut rig.app.history;
    let n = std::iter::from_fn(|| history.undo()).count();
    for _ in 0..n {
        history.redo();
    }
    n
}

#[test]
fn an_automation_take_keeps_its_lane_when_a_note_take_adds_a_notes_lane_before_it() {
    // The notes lane goes in before the automation lanes; the automation take's clip goes with its
    // lane rather than starting a second Level lane.
    let mut rig = song_rig();
    rig.at(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    // A knob's first recorded move, as `HoneydewApp::auto_record` makes it: its new Level lane, then the re-key.
    let before = rig.app.song.clone();
    let song = &mut rig.app.song;
    rig.app
        .auto_record
        .record(song, 1, LEVEL, (0.2, 0.5), BEAT, None)
        .unwrap();
    rig.app
        .rekey(&crate::app::tracks::lane_shifts(&before, &rig.app.song));
    rig.app
        .song_edited(crate::ui::views::SongEdit::Ongoing, BEAT);
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    let kinds = |rig: &Rig| {
        (rig.app.song.channels[1].lanes.iter())
            .map(|l| l.kind)
            .collect::<Vec<_>>()
    };
    use crate::song::LaneKind::{Automation, Notes};
    assert_eq!(kinds(&rig), [Notes, Notes, Automation]);
    let song = &mut rig.app.song;
    let added = rig
        .app
        .auto_record
        .record(song, 1, LEVEL, (0.5, 0.7), 3 * BEAT, None);
    assert_eq!(added, Some(None), "no new lane");
    assert_eq!(kinds(&rig), [Notes, Notes, Automation]);
}

#[test]
fn an_open_automation_editor_stays_open_when_a_note_take_adds_a_notes_lane() {
    // The editor's clip moved down a lane; it is not the replaced empty clip.
    let mut rig = song_rig();
    let lane = rig.app.song.channels[1].add_automation_lane(LEVEL);
    lane.add_clip(0, 4.0);
    lane.add_clip(8 * PPQ, 4.0);
    rig.app.song_edited(crate::ui::views::SongEdit::Final, 0);
    let key = crate::ui::preview::ClipKey {
        channel: 1,
        lane: 1,
        id: 2,
    };
    rig.app.navigate(Nav::EditClip(key));
    rig.frame(vec![]);
    assert_eq!(rig.app.nav.view, View::AutomationEditor(1));
    rig.at(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    let kinds: Vec<_> = (rig.app.song.channels[1].lanes.iter())
        .map(|l| l.kind)
        .collect();
    assert_eq!(kinds[1], crate::song::LaneKind::Notes, "{kinds:?}");
    assert_eq!(rig.app.nav.view, View::AutomationEditor(1));
    let current = crate::ui::preview::current(rig.app.nav.view, &rig.app.slots, &rig.app.song);
    assert_eq!(
        current,
        Some(crate::ui::preview::ClipKey { lane: 2, ..key })
    );
}

#[test]
fn disarming_with_automation_and_a_held_note_recorded_is_one_undo_step() {
    // The automation take's step closes with the notes'.
    let mut rig = song_rig();
    let undos = steps(&mut rig);
    rig.at(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    // A knob's first recorded move, as `HoneydewApp::auto_record` makes it: its new Level lane, then the re-key.
    let before = rig.app.song.clone();
    let song = &mut rig.app.song;
    rig.app
        .auto_record
        .record(song, 1, LEVEL, (0.2, 0.5), 3 * BEAT, None)
        .unwrap();
    rig.app
        .rekey(&crate::app::tracks::lane_shifts(&before, &rig.app.song));
    rig.app
        .song_edited(crate::ui::views::SongEdit::Ongoing, 3 * BEAT);
    rig.at(4 * BEAT);
    rig.take();
    rig.click(REC);
    assert!(rig.playing() && !rig.app.main_screen.armed);
    assert_eq!(steps(&mut rig), undos + 1, "one take, one undo step");
    let rebuilds = rig
        .take()
        .iter()
        .filter(|c| c.starts_with("ReplaceSong"))
        .count();
    assert_eq!(rebuilds, 1);
}

#[test]
fn an_engine_rebuilt_during_a_take_plays_without_the_takes_clip() {
    // Clips being recorded are left out of playback, whatever rebuilds the engine meanwhile.
    let mut rig = song_rig();
    rig.at(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.at(2 * BEAT);
    rig.type_key(egui::Key::Z, true);
    let take = |song: &Song| song.channels[1].lanes.get(1).map(|l| l.clips.len());
    assert_eq!(take(&rig.app.song), Some(1));
    rig.app
        .song_edited(crate::ui::views::SongEdit::Final, 2 * BEAT);
    assert_eq!(take(rig.app.audio.built_song().unwrap()), Some(0));
    // Once the take ends, the engine has it.
    rig.click(PLAY);
    assert_eq!(take(rig.app.audio.built_song().unwrap()), Some(1));
}

#[test]
fn a_release_while_armed_and_stopped_starts_nothing() {
    // Only a press starts playback.
    let mut rig = song_rig();
    rig.type_key(egui::Key::Z, true);
    rig.click(REC);
    rig.type_key(egui::Key::Z, false);
    rig.frame(vec![]);
    assert!(!rig.playing());
}

/// The Recorder's engine renders a quarter of a beat: its output's energy and channel 1's lit keys.
fn rendered(rig: &mut Rig) -> (f32, u128) {
    let mut output = vec![0.0; 6_000 * 2];
    let mut rec = rig.rec.0.borrow_mut();
    let engine = rec.engine.as_mut().expect("an engine");
    engine.render(&mut output, 2);
    (
        output.iter().map(|s| s * s).sum::<f32>(),
        engine.sounding(1),
    )
}

/// The key that starts playback keeps sounding: while REC is armed and the transport stopped, a press starts
/// playback in its own frame (no frame of delay), and its note goes out right after that Play, so the Play's
/// all-notes-off comes first: the key sounds and stays lit. A typed key, and a key of the on-screen piano.
#[test]
fn the_key_that_starts_playback_keeps_sounding() {
    for typed in [true, false] {
        let mut rig = song_rig();
        rig.app.settings.keyboard_kind = KeyboardMode::Classic as u8;
        rig.app.song_edited(SongEdit::Final, 0);
        rig.click(REC);
        rig.take();
        rendered(&mut rig);
        match typed {
            true => rig.type_key(egui::Key::Z, true),
            false => rig.press(pos2(2.5 * U, LOW)),
        }
        assert!(rig.playing(), "typed {typed}: started in the press's frame");
        let log = rig.transport();
        let on = log.iter().position(|c| c.starts_with("NoteOn 1 "));
        let key: u8 = (on.and_then(|at| log[at].split(' ').nth(2)?.parse().ok()))
            .unwrap_or_else(|| panic!("typed {typed}: {log:?}"));
        let play = log.iter().position(|c| c == "Play");
        assert!(play.is_some() && play < on, "typed {typed}: {log:?}");
        assert_eq!(rig.app.sounding(1), 1 << key, "typed {typed}: lit");
        let (energy, lit) = rendered(&mut rig);
        assert!(energy > 0.01, "typed {typed}: sounds");
        assert_eq!(lit, 1 << key);
    }
}

/// A live key held when Play is tapped: Play's all-notes-off cuts it, and its keyboard light goes out with
/// it, though the key is still down. Nothing strikes it again; its release sends only its note-off.
#[test]
fn a_key_held_at_play_goes_dark_and_its_release_sends_only_its_note_off() {
    let mut rig = song_rig();
    rig.type_key(egui::Key::Z, true);
    assert_eq!(rig.app.sounding(1), 1 << 48);
    rig.take();
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(rig.app.sounding(1), 0, "dark");
    rig.type_key(egui::Key::Z, false);
    assert_eq!(rig.transport(), ["Play", "NoteOff 1 48"]);
    rig.type_key(egui::Key::Z, true);
    assert_eq!(rig.app.sounding(1), 1 << 48, "struck again, lit again");
}

/// A frame that stops the recording transport (Space) and holds a typed press starts nothing: only a
/// press while armed and stopped as the frame starts starts playback.
#[test]
fn space_stopping_a_recording_with_a_typed_press_in_its_frame_stays_stopped() {
    let mut rig = song_rig();
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.playing());
    rig.take();
    let space = |pressed| super::typed::typed(egui::Key::Space, pressed, egui::Modifiers::NONE);
    rig.frame(vec![
        space(true),
        space(false),
        super::typed::typed(egui::Key::Z, true, egui::Modifiers::NONE),
    ]);
    assert!(!rig.playing());
    let log = rig.transport();
    assert!(
        log.contains(&"Stop".to_owned()) && !log.contains(&"Play".to_owned()),
        "{log:?}"
    );
}

/// Arming (Alt+R) and a typed press in one frame: the press sounds at once and starts nothing.
#[test]
fn arming_with_a_typed_press_in_its_frame_sounds_it_and_starts_nothing() {
    let mut rig = song_rig();
    rig.take();
    let z = super::typed::typed(egui::Key::Z, true, egui::Modifiers::NONE);
    rig.frame(vec![
        super::typed::typed(egui::Key::R, true, egui::Modifiers::ALT),
        super::typed::typed(egui::Key::R, false, egui::Modifiers::ALT),
        z,
    ]);
    assert!(rig.app.main_screen.armed);
    assert!(!rig.playing());
    assert_eq!(rig.transport(), ["NoteOn 1 48 89"]);
}

/// A ruler tap and a press that starts playback (REC armed) in one frame: the transport plays from the
/// tapped beat.
#[test]
fn a_ruler_tap_with_a_starting_press_in_its_frame_seeks_and_plays() {
    let mut rig = song_rig();
    rig.click(REC);
    rig.press(ruler(3.0));
    rig.take();
    rig.frame(vec![
        super::button_event(ruler(3.0), false),
        super::typed::typed(egui::Key::Z, true, egui::Modifiers::NONE),
    ]);
    assert!(rig.playing());
    let log = rig.transport();
    let at = |c: &str| log.iter().position(|s| s == c);
    assert!(
        at("Play").is_some() && at("Play") < at("Seek 384"),
        "{log:?}"
    );
    assert_eq!(rig.app.main_screen.play_start, 3 * BEAT);
}

/// Play start clears the live-held marks. A key held across the Play that starts a recording is cut, dark
/// and silent, and the take records nothing of it; its release sends only its note-off.
#[test]
fn a_key_held_across_the_play_that_starts_a_recording_is_not_recorded() {
    let mut rig = song_rig();
    rig.type_key(egui::Key::Z, true);
    rig.click(REC);
    rig.at(2 * BEAT);
    rig.click(PLAY);
    assert!(rig.playing());
    assert_eq!(rig.app.sounding(1), 0, "dark");
    rig.at(4 * BEAT);
    rig.frame(vec![]);
    rig.take();
    rig.type_key(egui::Key::Z, false);
    assert_eq!(rig.transport(), ["NoteOff 1 48"]);
    rig.at(5 * BEAT);
    rig.click(PLAY);
    let notes = (rig.app.song.channels[1].lanes.iter())
        .flat_map(|l| l.clips.iter())
        .flat_map(|c| c.events.iter())
        .filter(|e| matches!(e, crate::song::Event::Note { .. }))
        .count();
    assert_eq!(notes, 0);
}

/// Presses queued with Play start go out after its reset. With REC off, a press in the frame of Space or of a
/// Play tap goes out after that Play: it sounds and stays lit.
#[test]
fn a_press_in_the_frame_of_a_play_start_goes_out_after_the_play() {
    let z = super::typed::typed(egui::Key::Z, true, egui::Modifiers::NONE);
    for space in [true, false] {
        let mut rig = song_rig();
        // An engine to render.
        rig.app.song_edited(SongEdit::Final, 0);
        rig.frame(vec![]);
        rig.take();
        if space {
            let key =
                |pressed| super::typed::typed(egui::Key::Space, pressed, egui::Modifiers::NONE);
            rig.frame(vec![key(true), key(false), z.clone()]);
        } else {
            rig.press(PLAY);
            rig.take();
            rig.frame(vec![super::button_event(PLAY, false), z.clone()]);
        }
        assert!(rig.playing(), "{space}");
        let log = rig.transport();
        let at = |c: &str| log.iter().position(|s| s.starts_with(c));
        assert!(
            at("Play").is_some() && at("Play") < at("NoteOn 1 48 "),
            "{space}: {log:?}"
        );
        assert_eq!(rig.app.sounding(1), 1 << 48, "{space}: lit");
        let (energy, lit) = rendered(&mut rig);
        assert!(energy > 0.01, "{space}: sounds");
        assert_eq!(lit, 1 << 48, "{space}");
    }
}

/// A new song's channels, added as the add-channel popup adds them (each with the one-bar clip at the song start),
/// play to the song end, bar 2, and wrap to the song start. A clip made two bars long while playing moves the end to
/// bar 3.
#[test]
fn a_new_songs_channels_play_to_the_song_end_and_wrap_to_the_start() {
    let mut rig = Rig::new();
    // A press starts the audio.
    rig.click(OUTSIDE);
    for kind in [
        ChannelKind::Instrument,
        ChannelKind::Instrument,
        ChannelKind::Drums,
    ] {
        rig.app.add_channel(kind, 0);
        rig.frame(escape());
        rig.frame(vec![]);
    }
    let clips: Vec<_> = (rig.app.song.channels[1..].iter())
        .map(|c| (c.lanes[0].clips.len(), c.lanes[0].clips[0].start_tick))
        .collect();
    assert_eq!(clips, [(1, 0); 3]);
    rig.click(PLAY);
    assert!(rig.playing());
    // The highest tick the engine reaches before it first goes back, rendering 10 ms at a time.
    let wrap = |rig: &mut Rig| {
        let mut rec = rig.rec.0.borrow_mut();
        let engine = rec.engine.as_mut().expect("an engine");
        let mut last = engine.transport_tick();
        for _ in 0..2_000 {
            engine.render(&mut [0.0; 480 * 2], 2);
            let now = engine.transport_tick();
            if now < last {
                return (last, now);
            }
            last = now;
        }
        panic!("no wrap by tick {last}");
    };
    let (last, now) = wrap(&mut rig);
    assert!(
        (4 * BEAT - 8..4 * BEAT).contains(&last),
        "wrapped at {last}"
    );
    assert!(now < 8, "to {now}");
    // An edit while playing re-estimates the length.
    rig.app.song.channels[2].lanes[0].clips[0].loop_len_beats = 8.0;
    rig.app.song_edited(SongEdit::Final, 0);
    rig.frame(vec![]);
    let (last, _) = wrap(&mut rig);
    assert!(
        (8 * BEAT - 8..8 * BEAT).contains(&last),
        "wrapped at {last}"
    );
}

/// Song length re-estimate: an edit that changes the song length clips the song
/// loop to the new end; one that keeps it, or grows it, clips nothing. A loop wholly past the end is left empty.
#[test]
fn an_edit_that_shortens_the_song_clips_its_loop_to_the_new_end() {
    let mut rig = song_rig();
    rig.app.song.play_loop = Some((2 * PPQ, 12 * PPQ));
    rig.app.song_edited(SongEdit::Final, 0);
    let clip = |rig: &mut Rig, beats: f64| {
        rig.app.song.channels[1].lanes[0].clips[0].loop_len_beats = beats;
        rig.app.song_edited(SongEdit::Final, 0);
        rig.app.song.play_loop
    };
    assert_eq!(clip(&mut rig, 6.0), Some((2 * PPQ, 8 * PPQ)), "two bars");
    assert_eq!(
        clip(&mut rig, 16.0),
        Some((2 * PPQ, 8 * PPQ)),
        "it grows back alone"
    );
    rig.app.song.play_loop = Some((2 * PPQ, 20 * PPQ));
    rig.app.song.channels[1].rack.add(32);
    rig.app.song_edited(SongEdit::Final, 0);
    assert_eq!(
        rig.app.song.play_loop,
        Some((2 * PPQ, 20 * PPQ)),
        "same length"
    );
    rig.app.song.play_loop = Some((10 * PPQ, 14 * PPQ));
    assert_eq!(clip(&mut rig, 4.0), None, "wholly past the end: none");
}

/// A song opened keeps its loop, even one past its own end; only an edit's re-estimate clips it.
#[test]
fn an_opened_song_keeps_a_loop_past_its_end() {
    let mut rig = song_rig();
    rig.app.song_edited(SongEdit::Final, 0);
    let mut song = rig.app.song.clone();
    song.channels[1].lanes[0].clips[0].loop_len_beats = 4.0;
    song.play_loop = Some((2 * PPQ, 12 * PPQ));
    rig.app.song = song;
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert_eq!(rig.app.song.play_loop, Some((2 * PPQ, 12 * PPQ)));
}

/// The re-estimate while playing: a playhead outside the clipped (non-empty) loop seeks to its start, a re-sync; one
/// inside plays on. Recording, the loop keeps its end.
#[test]
fn playing_outside_a_clipped_loop_seeks_to_its_start_and_a_recording_keeps_its_end() {
    for (tick, recording) in [(10, false), (5, false), (10, true)] {
        let mut rig = song_rig();
        rig.app.song.play_loop = Some((2 * PPQ, 12 * PPQ));
        rig.app.song_edited(SongEdit::Final, 0);
        if recording {
            rig.click(REC);
        }
        rig.click(PLAY);
        rig.at(tick * BEAT);
        rig.frame(vec![]);
        rig.take();
        rig.app.song.channels[1].lanes[0].clips[0].loop_len_beats = 8.0;
        rig.app.song_edited(SongEdit::Final, tick * BEAT);
        let end = if recording { 12 } else { 8 };
        let want = Some((2 * PPQ, end * PPQ));
        assert_eq!(rig.app.song.play_loop, want, "{tick} {recording}");
        let log = rig.transport();
        assert!(log[0].starts_with("ReplaceSong adopting"), "{log:?}");
        let seek: &[&str] = if tick == 10 && !recording {
            &["Seek 256"]
        } else {
            &[]
        };
        assert_eq!(log[1..], [&["Play"], seek].concat(), "{tick} {recording}");
    }
}
