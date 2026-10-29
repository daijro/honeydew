//! REC and live automation recording: REC arms,
//! any stop disarms, Play while armed counts in first, a control moved while armed starts the transport, and after the
//! count-in each change is a point in one automation clip, one undo step at Stop.

use honeydew_audio::device::AudioRenderer;

use super::*;
use crate::song::{AutomationTarget, LaneKind, PPQ};

/// REC, in the transport bar's slot 0 (two slots left of play).
const REC: Pos2 = pos2(585.0 - 2.0 * 1.75 * U, BAR_Y);
const BEAT: u64 = PPQ as u64;
/// Channel 1's Level and Mute: the mixer strip, instance 0.
const LEVEL: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 0,
};
const MUTE: AutomationTarget = AutomationTarget {
    instance_id: 0,
    param: 2,
};

impl Rig {
    /// The undo steps there are (undone, then redone back).
    fn undo_steps(&mut self) -> usize {
        let history = &mut self.app.history;
        let n = std::iter::from_fn(|| history.undo()).count();
        for _ in 0..n {
            history.redo();
        }
        n
    }

    /// The control points of channel 1's lanes automating `target`, by lane.
    fn recorded(&self, target: AutomationTarget) -> Vec<Vec<(u32, u32, u16)>> {
        let lanes = self.app.song.channels[1].lanes.iter();
        let lanes = lanes.filter(|l| l.kind == LaneKind::Automation && l.target == Some(target));
        lanes
            .map(|l| {
                let clips = l.clips.iter().flat_map(|c| {
                    let pts =
                        crate::song::points::points(c, crate::song::points::PointKey::Automation);
                    pts.into_iter()
                        .map(|p| (c.start_tick, p.tick, p.value))
                        .collect::<Vec<_>>()
                });
                clips.collect()
            })
            .collect()
    }

    /// Renders the Recorder's engine for `frames` frames (it runs a count-in), given the output's buffer size first as
    /// the device does. Returns the stereo output.
    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut output = vec![0.0; 2 * frames];
        let mut rec = self.rec.0.borrow_mut();
        let buffer = rec.buffer_frames;
        let engine = rec.engine.as_mut().expect("an engine");
        engine.set_buffer_frames(buffer);
        engine.render(&mut output, 2);
        output
    }
}

/// [`mixer_rig`] whose Recorder holds an engine (it counts in), the count-in setting `count_in`.
fn rec_rig(count_in: bool) -> Rig {
    let mut rig = mixer_rig();
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.settings.count_in = count_in;
    rig.frame(vec![]);
    rig.take();
    rig
}

#[test]
fn rec_arms_with_the_metronome_and_any_stop_disarms() {
    let mut rig = rec_rig(false);
    rig.click(REC);
    assert!(rig.app.main_screen.armed);
    assert_eq!(rig.take(), ["Metronome true 0.500"]);
    rig.click(REC);
    assert!(!rig.app.main_screen.armed, "REC toggles");
    assert_eq!(rig.take(), ["Metronome false 0.500"]);
    // Armed, then Play and Stop: the stop disarms.
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.app.main_screen.armed);
    rig.click(PLAY);
    assert!(!rig.app.main_screen.armed);
    assert_eq!(
        rig.take(),
        [
            "Metronome true 0.500",
            "Play",
            "Stop",
            "Metronome false 0.500"
        ]
    );
    // So does a new song.
    rig.click(REC);
    rig.app.song = Song::default();
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.frame(vec![]);
    assert!(!rig.app.main_screen.armed);
    assert!(rig.take().ends_with(&["Metronome false 0.500".to_owned()]));
}

/// `Metronome when recording` off: the take is silent, yet still a recording to the engine (it runs past the one-bar
/// song end). Switched on while armed, the beats click again.
#[test]
fn rec_with_the_metronome_setting_off_records_without_clicks() {
    let mut rig = rec_rig(false);
    rig.app.settings.rec_metronome = false;
    rig.frame(vec![]);
    rig.take();
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.app.recording());
    assert_eq!(rig.take(), ["Metronome true 0.500 silent", "Play"]);
    let loud = |output: &[f32]| output.iter().any(|s| s.abs() > 0.01);
    assert!(!loud(&rig.render(6 * 24_000)), "no clicks");
    let tick = rig.rec.0.borrow().engine.as_ref().unwrap().transport_tick();
    assert_eq!(tick, 6 * BEAT, "no wrap at the song end");
    rig.app.settings.rec_metronome = true;
    rig.frame(vec![]);
    assert_eq!(rig.take(), ["Metronome true 0.500"]);
    assert!(loud(&rig.render(24_000)), "beat 6 clicks");
}

#[test]
fn play_while_armed_counts_in_first_with_the_count_in_setting_on() {
    let mut rig = rec_rig(true);
    rig.click(PLAY);
    rig.click(PLAY);
    assert_eq!(rig.take(), ["Play", "Stop"], "not armed: no count-in");
    rig.click(REC);
    rig.click(PLAY);
    assert_eq!(rig.take(), ["Metronome true 0.500", "CountIn 4", "Play"]);
    assert!(rig.rec.0.borrow().engine.as_ref().unwrap().counting_in());
}

#[test]
fn an_armed_fader_drag_starts_playback_and_records_points_after_the_count_in_in_one_undo_step() {
    // The acceptance: arm, move a slider (it starts the transport), two beats of moves, stop.
    let mut rig = rec_rig(true);
    let undos = rig.undo_steps();
    rig.click(REC);
    rig.take();
    rig.press(FADER_1);
    rig.drag_to(FADER_1 + vec2(0.0, 8.0));
    assert!(
        rig.app.main_screen.playing,
        "the move started the transport"
    );
    // The move went to the engine live first, then the count-in and Play.
    let log = rig.take();
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(log[0].starts_with("SetParam 1 0 0 "), "{log:?}");
    assert_eq!(log[1..], ["CountIn 4", "Play"]);
    // Moves during the count-in record nothing.
    rig.drag_to(FADER_1 + vec2(0.0, 16.0));
    assert!(rig.app.audio.counting_in());
    assert_eq!(rig.recorded(LEVEL), Vec::<Vec<_>>::new());
    // Four beats at the new song's 120 BPM (48 kHz): the count-in ends. The playhead is at beat 5 from then on.
    rig.render(96_000);
    rig.rec.0.borrow_mut().tick = 5 * BEAT;
    rig.frame(vec![]);
    assert!(!rig.app.audio.counting_in());
    // The move during the count-in went out live too, and only that.
    let log = rig.take();
    assert!(
        log.len() == 1 && log[0].starts_with("SetParam 1 0 0 "),
        "{log:?}"
    );
    let mut ticks = Vec::new();
    for (i, y) in [24.0, 32.0, 40.0, 48.0, 48.0].into_iter().enumerate() {
        let tick = 5 * BEAT + i as u64 * BEAT / 2;
        rig.rec.0.borrow_mut().tick = tick;
        rig.drag_to(FADER_1 + vec2(0.0, y));
        ticks.push(tick as u32 - 4 * PPQ);
        // Mid-take each move still goes to the engine live, and nothing else does.
        let log = rig.take();
        let live =
            log.len() == usize::from(i < 4) && log.iter().all(|c| c.starts_with("SetParam 1 0 0 "));
        assert!(live, "{i}: {log:?}");
    }
    rig.release(FADER_1 + vec2(0.0, 48.0));
    rig.rec.0.borrow_mut().tick = 7 * BEAT;
    rig.frame(vec![]);
    // One clip at beat 4 (floor(5/4)·4), a point per change at playhead − clip start; the unmoved last frame adds none.
    let [lane] = &rig.recorded(LEVEL)[..] else {
        panic!("{:?}", rig.recorded(LEVEL))
    };
    let at: Vec<u32> = lane
        .iter()
        .map(|&(start, tick, _)| {
            assert_eq!(start, 4 * PPQ);
            tick
        })
        .collect();
    assert_eq!(at, ticks[..4]);
    let values: Vec<u16> = lane.iter().map(|p| p.2).collect();
    assert!(values.windows(2).all(|w| w[1] < w[0]), "{values:?}");
    let level = rig.app.song.channels[1].params.0[0];
    assert_eq!(*values.last().unwrap(), crate::song::points::raw(level));
    // A second gesture in the same take, released before Stop: its release closes no step of its own.
    rig.rec.0.borrow_mut().tick = 7 * BEAT + BEAT / 2;
    rig.press(FADER_1 + vec2(0.0, 48.0));
    rig.drag_to(FADER_1 + vec2(0.0, 64.0));
    rig.release(FADER_1 + vec2(0.0, 64.0));
    assert_eq!(rig.recorded(LEVEL)[0].len(), 5);
    let moved = rig.app.song.channels[1].params.0[0];
    // Edits are ongoing while recording: the engine is rebuilt at Stop only, and the whole take is one undo step.
    let log = rig.take();
    assert!(
        !log.is_empty() && log.iter().all(|c| c.starts_with("SetParam 1 0 0 ")),
        "{log:?}"
    );
    rig.click(PLAY);
    assert!(!rig.app.main_screen.armed);
    let log = rig.take();
    assert!(
        log.starts_with(&["Stop".to_owned()]) && log.contains(&"ReplaceSong adopting".to_owned()),
        "{log:?}"
    );
    assert_eq!(rig.undo_steps(), undos + 1);
    // Its undo takes the points away; the fader stays where it was moved.
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(rig.recorded(LEVEL), Vec::<Vec<_>>::new());
    assert_eq!(rig.app.song.channels[1].params.0[0], moved);
}

#[test]
fn a_toggle_recorded_while_rolling_steps_from_its_old_value() {
    let mut rig = rec_rig(false);
    rig.click(REC);
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 2 * BEAT;
    rig.frame(vec![]);
    // No count-in: the mute's press and its release frame (it turns on) record at once, at beat 2 of a clip at 0.
    rig.click(MUTE_1);
    assert_eq!(rig.app.song.channels[1].params.0[2], 1.0);
    let lane = &rig.app.song.channels[1].lanes;
    assert!(
        lane.iter().any(|l| l.label == "Mute"),
        "the lane's auto name"
    );
    assert_eq!(
        rig.recorded(MUTE),
        [vec![(0, 2 * PPQ, 0), (0, 2 * PPQ, 65534)]]
    );
    // Off again a beat later: from on.
    rig.rec.0.borrow_mut().tick = 3 * BEAT;
    rig.click(MUTE_1);
    assert_eq!(
        rig.recorded(MUTE),
        [vec![
            (0, 2 * PPQ, 0),
            (0, 2 * PPQ, 65534),
            (0, 3 * PPQ, 65534),
            (0, 3 * PPQ, 0)
        ]]
    );
}

#[test]
fn moves_while_not_armed_or_stopped_record_nothing_and_save_no_undo_step() {
    let mut rig = rec_rig(false);
    let undos = rig.undo_steps();
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 2 * BEAT;
    rig.click(MUTE_1);
    rig.click(PLAY);
    assert_eq!(rig.recorded(MUTE), Vec::<Vec<_>>::new());
    assert_eq!(rig.undo_steps(), undos, "r36 C");
}

#[test]
fn the_current_lane_of_another_channel_takes_no_recording() {
    // Channel 1 (Drums) has a Level lane (1) whose empty clip starts at 0; the playlist's selected clip is on lane 1 of
    // channel 2. That is no current lane of channel 1: the recording makes a new Level lane, not that empty clip.
    let mut rig = rec_rig(false);
    let empty = rig.app.song.channels[1].add_automation_lane(LEVEL);
    empty.add_clip(0, 4.0);
    let other = rig.app.song.channels[2].add_automation_lane(LEVEL);
    let id = other.add_clip(0, 4.0).id;
    rig.app.song_edited(SongEdit::Final, 0);
    let (lane_1, lane_2) = (
        rig.app.song.channels[1].lanes.len() - 1,
        rig.app.song.channels[2].lanes.len() - 1,
    );
    assert_eq!((lane_1, lane_2), (1, 1));
    rig.app.main_screen.clips.selected = Some(crate::ui::clips::ClipRef {
        channel: 2,
        lane: lane_2,
        id,
    });
    rig.click(REC);
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = BEAT;
    rig.press(FADER_1);
    rig.drag_to(FADER_1 + vec2(0.0, 16.0));
    rig.drag_to(FADER_1 + vec2(0.0, 32.0));
    rig.release(FADER_1 + vec2(0.0, 32.0));
    let recorded = rig.recorded(LEVEL);
    assert_eq!(recorded.len(), 2, "a new lane: {recorded:?}");
    assert_eq!(recorded[0], [], "the empty clip stays empty");
    assert!(!recorded[1].is_empty());
}

#[test]
fn an_undo_or_redo_while_armed_is_no_move_and_starts_nothing() {
    // The fader moved before REC (no undo step), then undos take its channel away and a redo brings it back with
    // that value: no change of the recent control, so no Play and no points.
    let mut rig = rec_rig(false);
    rig.press(FADER_1);
    rig.drag_to(FADER_1 + vec2(0.0, 16.0));
    rig.release(FADER_1 + vec2(0.0, 16.0));
    let moved = rig.app.song.channels[1].params.0[0];
    rig.click(REC);
    rig.take();
    for keys in [
        egui::Modifiers::NONE,
        egui::Modifiers::NONE,
        egui::Modifiers::SHIFT,
    ] {
        rig.frame(z(keys));
        rig.frame(vec![]);
    }
    assert_eq!(rig.app.song.channels[1].params.0[0], moved);
    assert!(!rig.app.main_screen.playing);
    let log = rig.take();
    assert!(!log.contains(&"Play".to_owned()), "{log:?}");
    assert_eq!(rig.recorded(LEVEL), Vec::<Vec<_>>::new());
}

// ---- Audio recording in the app ----

mod audio {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use super::*;
    use crate::recording::input::{InputLink, InputRing};
    use crate::ui::clips::LiveTake;

    /// 150 BPM at the Recorder's 48 kHz: 150 frames a tick, 19 200 a beat.
    const FRAMES_PER_TICK: usize = 150;
    const FRAMES_PER_BEAT: usize = FRAMES_PER_TICK * PPQ as usize;

    /// A rig that records: its audio runs (the Recorder holds an engine) with a 150 BPM song, its files are on disk
    /// in its own user folder, its input is a fake mono device, and an `Audio Record` channel (1) was just added.
    struct AudioRig {
        rig: Rig,
        user: PathBuf,
        /// The fake device's opens, and whether it fails to open.
        opens: Rc<Cell<usize>>,
        fail: Rc<Cell<bool>>,
    }

    impl AudioRig {
        fn new(name: &str, fail: bool) -> Self {
            let mut rig = Rig::new();
            rig.click(OUTSIDE);
            let user =
                std::env::temp_dir().join(format!("honeydew-h13-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&user);
            rig.app.files = ProjectFiles::on_disk(&user);
            let (opens, fail) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(fail)));
            let (o, f) = (opens.clone(), fail.clone());
            rig.app.input = InputLink::new(move |device, rate| {
                assert_eq!((device, rate), (None, 48_000));
                o.set(o.get() + 1);
                if f.get() {
                    return Err("denied".into());
                }
                let ring = Arc::new(InputRing::new(1, rate, 2 * rate as usize));
                Ok((ring, Box::new(()) as Box<dyn std::any::Any>))
            });
            rig.app.song.tempo_bpm = 150.0;
            rig.app.add_channel(ChannelKind::AudioRecord, 0);
            rig.frame(vec![]);
            rig.take();
            Self {
                rig,
                user,
                opens,
                fail,
            }
        }

        /// The Recorder's engine renders `frames` frames, as `frames` frames of `x` come into the open input.
        fn feed(&mut self, frames: usize, x: f32) {
            let ring = self.rig.app.input.ring().expect("an open input");
            ring.push(std::iter::repeat_n(x, frames));
            self.rig.render(frames);
        }

        /// One frame with the playhead at `tick`.
        fn at(&mut self, tick: u64) {
            self.rig.rec.0.borrow_mut().tick = tick;
            self.rig.frame(vec![]);
        }

        fn message(&self) -> Option<Vec<String>> {
            self.rig.app.nav.overlays.iter().find_map(|o| match o {
                Overlay::Message { lines, .. } => Some(lines.clone()),
                _ => None,
            })
        }
    }

    impl Drop for AudioRig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.user);
        }
    }

    #[test]
    fn an_audio_record_channel_is_record_and_its_id_arms_rec_and_opens_the_input_while_armed() {
        let mut a = AudioRig::new("arm", false);
        let ch = &a.rig.app.song.channels[1];
        assert_eq!(ch.name, format!("Record {}", ch.id));
        assert!(a.rig.app.main_screen.armed);
        assert_eq!(a.opens.get(), 1);
        // The engines monitor the ring the link opened.
        let ring = a.rig.app.input.ring().expect("open").clone();
        let monitored = a.rig.app.audio.input.as_ref().map(|i| i.ring.clone());
        assert!(monitored.is_some_and(|m| Arc::ptr_eq(&m, &ring)));
        assert_eq!(a.message(), None);
        // Disarmed, it closes; armed again, it opens.
        a.rig.click(REC);
        assert!(a.rig.app.input.ring().is_none() && a.rig.app.audio.input.is_none());
        a.rig.click(REC);
        assert!(a.rig.app.input.ring().is_some());
        assert_eq!(a.opens.get(), 2);
    }

    #[test]
    fn an_input_that_fails_shows_the_permission_message_until_rec_is_armed_again() {
        let mut a = AudioRig::new("denied", true);
        let permission = vec![MIC_PERMISSION.to_owned()];
        assert!(a.rig.app.main_screen.armed);
        assert_eq!(a.message(), Some(permission.clone()));
        assert_eq!(a.opens.get(), 1);
        // Not tried again every frame, nor asked again.
        a.rig.app.nav.overlays.clear();
        a.rig.frame(vec![]);
        assert_eq!((a.opens.get(), a.message()), (1, None));
        // Arming again tries again; adding another `Audio Record` channel asks again too.
        a.rig.click(REC);
        a.rig.click(REC);
        assert_eq!((a.opens.get(), a.message()), (2, Some(permission.clone())));
        a.rig.app.nav.overlays.clear();
        a.fail.set(false);
        a.rig.app.add_channel(ChannelKind::AudioRecord, 0);
        assert_eq!(a.opens.get(), 3);
        assert!(a.rig.app.input.ring().is_some() && a.message().is_none());
    }

    #[test]
    fn rec_in_a_song_without_an_audio_record_channel_opens_no_input() {
        let mut rig = rec_rig(false);
        rig.click(REC);
        rig.click(PLAY);
        rig.frame(vec![]);
        assert!(rig.app.input.ring().is_none() && rig.app.recorder.is_none());
        assert!(rig.app.nav.overlays.is_empty(), "no message");
    }

    #[test]
    fn a_recording_counts_in_grows_live_and_leaves_a_clip_and_its_file_with_no_undo_step() {
        // The acceptance: end to end in the rig, with no device.
        let mut a = AudioRig::new("take", false);
        let undos = a.rig.undo_steps();
        a.rig.app.files.saved();
        a.rig.click(PLAY);
        assert_eq!(a.rig.take(), ["CountIn 4", "Play"]);
        // The count-in (on by default): what comes in meanwhile is not recorded.
        a.feed(4_800, 0.25);
        a.at(0);
        assert!(a.rig.app.audio.counting_in());
        let takes = |a: &AudioRig| a.rig.app.recorder.as_ref().map(|r| r.takes().len());
        assert_eq!(takes(&a), Some(0));
        a.feed(4 * FRAMES_PER_BEAT - 4_800, 0.25);
        // A beat in: the take begins at the record start and grows live on the channel's empty audio track.
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(PPQ.into());
        a.at(PPQ.into());
        let bar = |end: f64| LiveTake {
            channel: 1,
            lane: 0,
            start: 0.0,
            end,
            clip: None,
        };
        assert_eq!(a.rig.app.main_screen.takes, [bar(PPQ.into())]);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(2 * BEAT);
        a.at(2 * BEAT);
        assert_eq!(a.rig.app.main_screen.takes, [bar(2.0 * f64::from(PPQ))]);
        assert!(!a.rig.app.files.modified());
        // Stop: a clip of the take's two beats, its file, a rebuilt engine and no undo step. REC disarms and the input
        // closes.
        a.rig.click(PLAY);
        let log = a.rig.take();
        assert!(
            log.contains(&"Stop".to_owned()) && log.contains(&"ReplaceSong adopting".to_owned()),
            "{log:?}"
        );
        assert!(!a.rig.app.main_screen.armed && a.rig.app.input.ring().is_none());
        assert!(a.rig.app.recorder.is_none());
        // The bars are drawn before the frame takes the input: the next frame has none.
        a.rig.frame(vec![]);
        assert_eq!(a.rig.app.main_screen.takes, []);
        let clips = &a.rig.app.song.channels[1].lanes[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(
            (clips[0].start_tick, clips[0].file.source_len_beats),
            (0, 2.0)
        );
        assert_eq!(clips[0].file.sample.as_ref().unwrap().name, "Rec");
        let file = a.user.join("My Recordings").join("Rec.wav");
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes.len(), 46 + 2 * 2 * FRAMES_PER_BEAT);
        // Only the input after the count-in: 0.5, trunc(0.5·32767).
        let samples = bytes[46..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]));
        assert!(samples.into_iter().all(|x| x == 16_383));
        assert_eq!(a.rig.undo_steps(), undos);
        assert!(!a.rig.app.history.can_redo());
        // The take sets the song-changed flag.
        assert!(a.rig.app.files.modified());
    }

    #[test]
    fn starting_a_recording_turns_preview_off() {
        let mut a = AudioRig::new("preview", false);
        let lane = &mut a.rig.app.song.channels[1].lanes[0];
        let id = lane.add_clip(16 * PPQ, 4.0).id;
        a.rig.app.song_edited(SongEdit::Final, 0);
        let key = ClipKey {
            channel: 1,
            lane: 0,
            id,
        };
        // Its clip is the current one, which keeps Preview on: the recording's start turns it off.
        a.rig.app.navigate(Nav::EditClip(key));
        let ctx = a.rig.ctx.clone();
        a.rig.app.preview_on(&ctx, key, 0);
        assert_eq!(a.rig.app.preview, Some(key));
        a.rig.frame(vec![]);
        assert_eq!(a.rig.app.preview, Some(key));
        a.rig.take();
        a.rig.click(PLAY);
        assert_eq!(a.rig.app.preview, None);
        // The count-in starts; the engine for the whole song then takes over (Preview off: all notes off, a re-sync)
        // with the count-in running on in it.
        let log = [
            "CountIn 4",
            "Play",
            "ReplaceSong adopting silencing 0,1",
            "Play",
        ];
        assert_eq!(a.rig.take(), log);
        assert!(a.rig.app.audio.counting_in());
    }

    #[test]
    fn an_armed_channel_shows_its_dot_with_multitrack_recording_on() {
        let mut a = AudioRig::new("dot", false);
        let header = egui::Rect::from_min_size(pos2(0.0, RULER + 2.0 * U), vec2(1.5 * U, 2.0 * U));
        let center = header.left_top() + vec2(0.3 * U, 0.3 * U);
        // The channel is current, so its cell is filled with its colour and the dot takes the icon's `moduleFill`.
        let ink = a.rig.app.theme.colors.module_fill.to_color32();
        let dots = |rig: &Rig| -> Vec<(f32, egui::Color32)> {
            (rig.out.shapes.iter())
                .filter_map(|s| match &s.shape {
                    egui::Shape::Circle(c) if c.center == center => Some((c.radius, c.fill)),
                    _ => None,
                })
                .collect()
        };
        for (multitrack, armed) in [(false, true), (true, false), (true, true)] {
            a.rig.app.settings.multitrack_recording = multitrack;
            a.rig.app.song.channels[1].file.record_armed = armed;
            a.rig.frame(vec![]);
            let want = if multitrack && armed {
                vec![(0.1 * U, ink)]
            } else {
                vec![]
            };
            assert_eq!(dots(&a.rig), want, "multitrack {multitrack}, armed {armed}");
        }
    }

    /// The takes of the recording under way, as (lane, start tick, frames).
    fn takes(a: &AudioRig) -> Vec<(usize, u64, u64)> {
        let recorder = a.rig.app.recorder.as_ref().expect("a recording");
        (recorder.takes().iter())
            .map(|t| (t.lane, t.start_tick, t.frames))
            .collect()
    }

    #[test]
    fn a_tempo_change_mid_take_keeps_the_take_and_its_live_bar_with_the_playhead() {
        // The tempo editor opens over a running take (it is modal for touches, not for the transport), so the
        // take must follow a new tempo: 1 beat at 150 BPM, then 2 at 300 (9 600 frames a beat).
        let mut a = AudioRig::new("tempo", false);
        a.rig.click(PLAY);
        a.feed(4 * FRAMES_PER_BEAT, 0.0);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(BEAT);
        a.at(BEAT);
        a.rig.click(BPM);
        assert_eq!(a.rig.overlays(), [Overlay::TempoEditor]);
        assert!(a.rig.app.main_screen.playing && a.rig.app.recorder.is_some());
        a.rig.frame(escape());
        a.rig.app.song.set_tempo(300.0);
        a.rig.app.song_edited(SongEdit::Final, BEAT);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(3 * BEAT);
        a.at(3 * BEAT);
        assert_eq!(takes(&a), [(0, 0, 2 * FRAMES_PER_BEAT as u64)]);
        let bar = LiveTake {
            channel: 1,
            lane: 0,
            start: 0.0,
            end: 3.0 * f64::from(PPQ),
            clip: None,
        };
        assert_eq!(a.rig.app.main_screen.takes, [bar]);
        // The clip is the file's length at the tempo it stops at: 38 400 frames at 300 BPM.
        a.rig.click(PLAY);
        let clips = &a.rig.app.song.channels[1].lanes[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(
            (clips[0].start_tick, clips[0].file.source_len_beats),
            (0, 4.0)
        );
    }

    #[test]
    fn a_take_at_the_decode_cap_ends_the_recording_with_a_message_and_a_clip_that_loads_back() {
        // with the cap at one beat of frames. REC disarms and the transport plays on.
        let mut a = AudioRig::new("cap", false);
        a.rig.click(PLAY);
        a.rig.app.recorder =
            (a.rig.app.recorder.take()).map(|r| r.with_max_frames(FRAMES_PER_BEAT as u64));
        a.feed(4 * FRAMES_PER_BEAT, 0.0);
        a.feed(FRAMES_PER_BEAT / 2, 0.5);
        a.at(PPQ as u64 / 2);
        assert_eq!(a.message(), None);
        a.rig.take();
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(3 * BEAT / 2);
        let message = "Recording stopped. A take can be at most 1 minutes long.";
        assert_eq!(a.message(), Some(vec![message.to_owned()]));
        assert!(!a.rig.app.main_screen.armed && a.rig.app.main_screen.playing);
        // The frame itself leaves the clip (the cap's frames) and the file; the next tells the engine the metronome is
        // off.
        assert!(a.rig.app.recorder.is_none());
        a.at(3 * BEAT / 2);
        assert!(a.rig.take().contains(&"Metronome false 0.500".to_owned()));
        let clips = &a.rig.app.song.channels[1].lanes[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file.source_len_beats, 1.0);
        let file = a.user.join("My Recordings").join("Rec.wav");
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes.len(), 46 + 2 * FRAMES_PER_BEAT);
        let loaded = honeydew_audio::sample::decode_with_limit(&bytes, FRAMES_PER_BEAT).unwrap();
        assert_eq!(loaded.frames(), FRAMES_PER_BEAT);
        // Only the one take (and the folder's `.nomedia`).
        let mut names: Vec<_> = (a.user.join("My Recordings").read_dir().unwrap())
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [".nomedia", "Rec.wav"]);
        // REC armed again (the message closed first), while the transport plays, records anew.
        a.rig.app.nav.overlays.clear();
        a.rig.click(REC);
        assert!(a.rig.app.recorder.is_some());
    }

    #[test]
    fn rec_pressed_in_the_cap_frame_and_released_in_the_next_still_leaves_the_capped_clip() {
        // The finger goes down on REC in the frame the take reaches the cap, and lifts in the next: the click gets
        // through the message and arms REC again. The capped recording ended in its own frame, so its clip is there.
        let mut a = AudioRig::new("cap-press", false);
        a.rig.click(PLAY);
        a.rig.app.recorder =
            (a.rig.app.recorder.take()).map(|r| r.with_max_frames(FRAMES_PER_BEAT as u64));
        a.feed(4 * FRAMES_PER_BEAT, 0.0);
        a.feed(FRAMES_PER_BEAT / 2, 0.5);
        a.at(PPQ as u64 / 2);
        a.rig.frame(vec![egui::Event::PointerMoved(REC)]);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.rig.rec.0.borrow_mut().tick = 3 * BEAT / 2;
        a.rig.button(REC, true);
        a.rig.button(REC, false);
        assert!(a.rig.app.main_screen.armed, "the click armed REC again");
        let clips = &a.rig.app.song.channels[1].lanes[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file.source_len_beats, 1.0);
        let bytes = std::fs::read(a.user.join("My Recordings").join("Rec.wav")).unwrap();
        assert_eq!(bytes.len(), 46 + 2 * FRAMES_PER_BEAT);
    }

    #[test]
    fn a_tempo_drag_under_way_leaves_the_take_at_the_tempo_the_engine_plays_at() {
        // The dial sets the engine's tempo live (`SongEdit::Ongoing`):
        // the playhead the take follows runs at each new tempo, and the take goes on as one. 150 BPM is 64 ticks for
        // 9 600 frames.
        let mut a = AudioRig::new("drag", false);
        a.rig.click(PLAY);
        a.feed(4 * FRAMES_PER_BEAT, 0.0);
        let step = |a: &mut AudioRig, tick: u64, frames: usize, bpm: f64| {
            a.feed(frames, 0.5);
            let tick = tick + (frames as f64 * bpm / 150.0) as u64 / FRAMES_PER_TICK as u64;
            a.at(tick);
            tick
        };
        let mut tick = step(&mut a, 0, FRAMES_PER_BEAT, 150.0);
        // Down, then up: each for two polls.
        for bpm in [75.0, 75.0, 300.0, 300.0] {
            a.rig.app.song.set_tempo(bpm);
            a.rig.app.song_edited(SongEdit::Ongoing, tick);
            assert_eq!(a.rig.app.audio.engine_tempo(&a.rig.app.song), bpm);
            tick = step(&mut a, tick, FRAMES_PER_BEAT / 2, bpm);
        }
        // The drag ends: the engine plays on at 300 BPM, 9 600 frames a beat.
        a.rig.app.song_edited(SongEdit::Final, tick);
        a.feed(FRAMES_PER_BEAT / 2, 0.5);
        tick += BEAT;
        a.at(tick);
        // The bars are drawn before the frame takes the input.
        a.at(tick);
        let frames = (FRAMES_PER_BEAT + 4 * FRAMES_PER_BEAT / 2 + FRAMES_PER_BEAT / 2) as u64;
        assert_eq!(takes(&a), [(0, 0, frames)]);
        let ends: Vec<_> = a.rig.app.main_screen.takes.iter().map(|t| t.end).collect();
        assert_eq!(ends, [tick as f64]);
    }

    #[test]
    fn a_take_after_a_count_in_starts_where_the_engine_holds_the_transport() {
        // An engine seek during the count-in moves the held position: the take
        // starts there once the count-in ends, and the input meanwhile is not recorded.
        let mut a = AudioRig::new("count-in", false);
        a.rig.click(PLAY);
        assert!(a.rig.app.audio.counting_in());
        a.feed(4_800, 0.25);
        a.rig
            .rec
            .0
            .borrow_mut()
            .engine
            .as_mut()
            .unwrap()
            .seek(2 * BEAT);
        a.feed(4 * FRAMES_PER_BEAT - 4_800, 0.25);
        a.at(2 * BEAT);
        assert_eq!(takes(&a), []);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(3 * BEAT);
        assert_eq!(takes(&a), [(0, 2 * BEAT, FRAMES_PER_BEAT as u64)]);
    }

    #[test]
    fn arming_during_playback_records_from_the_playhead_with_no_count_in() {
        let mut a = AudioRig::new("punch", false);
        a.rig.click(REC);
        a.rig.click(PLAY);
        a.rig.render(2 * FRAMES_PER_BEAT);
        a.at(2 * BEAT);
        a.rig.take();
        // The input opens, and an edit's engine that records it takes over: the transport goes on, with no count-in.
        a.rig.click(REC);
        let log = a.rig.take();
        assert!(log.contains(&"Metronome true 0.500".to_owned()), "{log:?}");
        assert!(
            log.iter().any(|c| c.starts_with("ReplaceSong adopting")),
            "{log:?}"
        );
        assert!(
            !log.iter()
                .any(|c| c.starts_with("CountIn") || c.starts_with("Seek")),
            "{log:?}"
        );
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(3 * BEAT);
        assert_eq!(takes(&a), [(0, 2 * BEAT, FRAMES_PER_BEAT as u64)]);
        // A playhead behind the running take's start (the frame after a loop wrap, before the recorder sees the next
        // take) ends its bar where its frames reach, never before its start.
        a.at(BEAT);
        let bars: Vec<_> = a
            .rig
            .app
            .main_screen
            .takes
            .iter()
            .map(|t| (t.start, t.end))
            .collect();
        assert_eq!(bars, [(2.0 * BEAT as f64, 3.0 * BEAT as f64)]);
    }

    #[test]
    fn without_a_calibration_a_take_drops_the_engines_buffer_size_and_with_the_toggle_on_nothing() {
        // L = the engine's audio buffer size, 256 frames here, while compensating (the toggle unset or OFF); nothing
        // with the toggle ON.
        for (toggle, dropped) in [(None, 256), (Some(false), 256), (Some(true), 0)] {
            let mut a = AudioRig::new("buffer", false);
            a.rig.rec.0.borrow_mut().buffer_frames = 256;
            a.rig.app.settings.latency_compensation = toggle;
            a.rig.app.settings.count_in = false;
            a.rig.frame(vec![]);
            a.rig.click(PLAY);
            a.feed(FRAMES_PER_BEAT, 0.5);
            a.at(BEAT);
            let frames = (FRAMES_PER_BEAT - dropped) as u64;
            assert_eq!(takes(&a), [(0, 0, frames)], "{toggle:?}");
            // The live bar ends at the playhead, with no latency offset (the bars are drawn before the frame polls).
            a.at(BEAT);
            let ends: Vec<_> = a.rig.app.main_screen.takes.iter().map(|t| t.end).collect();
            assert_eq!(ends, [BEAT as f64], "{toggle:?}");
        }
    }

    #[test]
    fn a_track_the_stop_adds_moves_the_selection_and_the_editor_on_the_tracks_after_it() {
        // Record 1: its audio track's clip covers the record start, so the take needs a new audio track, which goes
        // before the Level automation track that holds the selected clip, open in the automation editor.
        let mut a = AudioRig::new("rekey", false);
        let ch = &mut a.rig.app.song.channels[1];
        ch.lanes[0].add_clip(0, 4.0);
        let id = ch.add_automation_lane(LEVEL).add_clip(0, 4.0).id;
        a.rig.app.song_edited(SongEdit::Final, 0);
        let key = ClipKey {
            channel: 1,
            lane: 1,
            id,
        };
        a.rig.app.main_screen.clips.selected = Some(crate::ui::clips::ClipRef {
            channel: 1,
            lane: 1,
            id,
        });
        a.rig.app.navigate(Nav::EditClip(key));
        a.rig.frame(vec![]);
        assert_eq!(a.rig.app.nav.view, View::AutomationEditor(1));
        a.rig.click(PLAY);
        a.feed(4 * FRAMES_PER_BEAT, 0.0);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(BEAT);
        assert_eq!(takes(&a), [(1, 0, FRAMES_PER_BEAT as u64)]);
        a.rig.click(PLAY);
        let kinds: Vec<LaneKind> = (a.rig.app.song.channels[1].lanes.iter())
            .map(|l| l.kind)
            .collect();
        assert_eq!(
            kinds,
            [LaneKind::Audio, LaneKind::Audio, LaneKind::Automation]
        );
        assert_eq!(a.rig.app.song.channels[1].lanes[1].clips.len(), 1);
        // Both follow the automation track to 2.
        let selected = a.rig.app.main_screen.clips.selected.map(|r| (r.lane, r.id));
        assert_eq!(selected, Some((2, id)));
        assert_eq!(
            a.rig.app.slots.automation_editor.clips.get(&1),
            Some(&(2, id))
        );
        a.rig.frame(vec![]);
        assert_eq!(a.rig.app.nav.view, View::AutomationEditor(1));
    }

    #[test]
    fn an_input_reopened_mid_take_records_on_into_a_new_clip_at_the_reopen() {
        // The stream fails mid-take and the next frame reopens the device on a new ring of the same width and rate. It
        // keeps the tap the recorder reads, and the engine built with the new ring starts a take on it (S = L, 0 here),
        // so the recorder opens a clip there and nothing is lost.
        let mut a = AudioRig::new("reopen", false);
        a.rig.app.settings.count_in = false;
        a.rig.frame(vec![]);
        a.rig.click(PLAY);
        a.feed(FRAMES_PER_BEAT, 0.5);
        a.at(BEAT);
        a.rig.app.input.ring().expect("open").set_failed();
        a.at(BEAT);
        assert_eq!(a.opens.get(), 2, "reopened");
        a.feed(2 * FRAMES_PER_BEAT, 0.5);
        a.at(3 * BEAT);
        let beat = FRAMES_PER_BEAT as u64;
        assert_eq!(takes(&a), [(0, 0, beat), (0, BEAT, 2 * beat)]);
    }

    /// The disarm race: REC disarmed while recording past the old song end (one bar here). The take's
    /// clip makes the song two bars long, and its engine goes out before the disarm: an audio callback between them
    /// would otherwise find the old engine disarmed past its end and wrap it to 0. The transport plays on.
    #[test]
    fn a_disarm_past_the_old_song_end_reaches_the_engine_after_the_takes_longer_song() {
        let mut a = AudioRig::new("disarm", false);
        a.rig.app.settings.count_in = false;
        a.rig.frame(vec![]);
        a.rig.click(PLAY);
        a.feed(5 * FRAMES_PER_BEAT, 0.5);
        a.at(5 * BEAT);
        a.rig.take();
        a.rig.click(REC);
        assert!(!a.rig.app.main_screen.armed);
        let log = a.rig.take();
        let at = |c: &str| log.iter().position(|s| s.starts_with(c));
        let (song, disarm) = (at("ReplaceSong"), at("Metronome false"));
        assert!(song.is_some() && song < disarm, "{log:?}");
        a.rig.render(FRAMES_PER_BEAT);
        let tick = a
            .rig
            .rec
            .0
            .borrow()
            .engine
            .as_ref()
            .unwrap()
            .transport_tick();
        assert!(tick > 5 * BEAT, "plays on: {tick}");
    }
}

// ---- Live recording on a drum channel ----

/// Channel 1's (Drums) notes lanes (kind 3, flag 1), by lane: their hits as (clip start, pos, pad, length in beats).
fn drum_hits(rig: &Rig) -> Vec<Vec<(u32, u32, u8, f64)>> {
    let lanes = rig.app.song.channels[1].lanes.iter();
    let notes =
        lanes.filter(|l| l.kind == LaneKind::Drums && l.grid == crate::song::DrumGrid::Notes);
    notes
        .map(|l| {
            let hits = l.clips.iter().flat_map(|c| {
                c.events.iter().filter_map(|e| match *e {
                    crate::song::Event::Drum {
                        pos,
                        len_beats,
                        pad,
                        velocity,
                        ..
                    } => {
                        assert_eq!(velocity, 255, "pads play at 1.0");
                        Some((c.start_tick, pos, pad, len_beats))
                    }
                    _ => None,
                })
            });
            hits.collect()
        })
        .collect()
}

/// [`rec_rig`] with the Drums channel (1) current, its pads in the keyboard band, the stopped playhead at beat 5.
fn drum_rig() -> Rig {
    let mut rig = rec_rig(false);
    rig.app.selected_channel = 1;
    rig.rec.0.borrow_mut().tick = 5 * BEAT;
    rig.frame(vec![]);
    rig
}

#[test]
fn pad_hits_while_recording_go_to_a_new_notes_lane_with_their_held_length_in_one_undo_step() {
    let mut rig = drum_rig();
    // Not recording: a hit only plays.
    rig.click(pad_centre(0));
    assert!(rig.take().contains(&"NoteOn 1 48 127".to_owned()));
    assert_eq!(drum_hits(&rig), Vec::<Vec<_>>::new());
    let (undos, lanes) = (rig.undo_steps(), rig.app.song.channels[1].lanes.len());
    // The take starts at beat 5: its clip at floor(5/4)·4 = 4.
    rig.click(REC);
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 6 * BEAT;
    rig.press(pad_centre(0));
    rig.rec.0.borrow_mut().tick = 6 * BEAT + BEAT / 2;
    rig.release(pad_centre(0));
    // A tap: released as soon as pressed, so the shortest length.
    rig.rec.0.borrow_mut().tick = 7 * BEAT;
    rig.click(pad_centre(1));
    let (four, ppq) = (4 * PPQ, PPQ);
    assert_eq!(
        drum_hits(&rig),
        [vec![(four, 2 * ppq, 0, 0.5), (four, 3 * ppq, 1, 0.01)]]
    );
    // The step lane had no flag-1 lane to reuse: a new one after it.
    let ch = &rig.app.song.channels[1];
    assert_eq!(ch.lanes.len(), lanes + 1);
    assert_eq!(ch.lanes[1].grid, crate::song::DrumGrid::Notes);
    // Stop: the engine gets the hits with no re-sync (a note take only adds them), and the take is one undo step.
    rig.take();
    rig.click(PLAY);
    let log = rig.take();
    let quiet = "ReplaceSong adopting no-resync".to_owned();
    assert!(log.contains(&quiet), "{log:?}");
    assert_eq!(rig.undo_steps(), undos + 1);
    rig.frame(z(egui::Modifiers::NONE));
    assert_eq!(drum_hits(&rig), Vec::<Vec<_>>::new());
    assert_eq!(rig.app.song.channels[1].lanes.len(), lanes);
}

#[test]
fn a_drum_take_reuses_the_first_notes_lane_with_no_clip_at_its_start() {
    let mut rig = drum_rig();
    let ch = &mut rig.app.song.channels[1];
    // Three notes lanes: the first has a clip over beat 4 (the take's start, rounded down); the second and third are
    // free there, and the first of those takes it.
    for _ in 0..3 {
        let lane = ch.add_lane(LaneKind::Drums);
        ch.lanes[lane].grid = crate::song::DrumGrid::Notes;
    }
    ch.lanes[1].add_clip(2 * PPQ, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    let lanes = rig.app.song.channels[1].lanes.len();
    rig.click(REC);
    rig.click(PLAY);
    rig.rec.0.borrow_mut().tick = 5 * BEAT + BEAT / 4;
    rig.click(pad_centre(2));
    assert_eq!(rig.app.song.channels[1].lanes.len(), lanes, "no new lane");
    let (four, ppq) = (4 * PPQ, PPQ);
    assert_eq!(
        drum_hits(&rig),
        [vec![], vec![(four, ppq + ppq / 4, 2, 0.01)], vec![]]
    );
}

/// The level ring around pad `pad` (its centre, size r + 2·ceil(U/16), seen at trunc(1.5 size)/√2): its fill.
fn glow(rig: &Rig, pad: usize) -> egui::Color32 {
    let r = crate::ui::drums::pad_grid(
        egui::Rect::from_min_max(pos2(0.0, KEYS_TOP), pos2(1170.0, 540.0)),
        1170.0,
        U,
    )
    .r;
    (rig.out.shapes.iter())
        .find_map(|s| match &s.shape {
            egui::Shape::Path(p) if p.points.len() == 6 => {
                let centre = p
                    .points
                    .iter()
                    .fold(egui::Vec2::ZERO, |a, q| a + q.to_vec2())
                    / 6.0;
                let top = p.points.iter().map(|q| q.y).fold(f32::INFINITY, f32::min);
                let visible = (1.5 * (r + 4.0)).trunc() * std::f32::consts::FRAC_1_SQRT_2;
                ((centre.to_pos2() - pad_centre(pad)).length() <= 1.0
                    && (centre.y - top - visible).abs() < 0.5)
                    .then_some(p.fill)
            }
            _ => None,
        })
        .expect("a level ring")
}

#[test]
fn a_hit_on_a_rearranged_pad_records_its_slots_row_and_that_pad_glows() {
    // KP's `PADS` map: pads 0 and 4 swapped, so pad 4 plays slot 0 (key 48) and pad 0 slot 4.
    use honeydew_audio::MeterSnapshot;
    use honeydew_audio::device::{ModuleKey, ModuleMeter};
    let mut rig = drum_rig();
    let kit = rig.app.song.channels[1].drum_kit.as_mut().unwrap();
    kit.file.pads.swap(0, 4);
    let id = kit.slots[0].file.id;
    rig.click(REC);
    rig.click(PLAY);
    rig.take();
    rig.rec.0.borrow_mut().tick = 6 * BEAT;
    rig.click(pad_centre(4));
    assert!(rig.take().contains(&"NoteOn 1 48 127".to_owned()));
    let (four, ppq) = (4 * PPQ, PPQ);
    assert_eq!(
        drum_hits(&rig),
        [vec![(four, 2 * ppq, 0, 0.01)]],
        "slot 0's row"
    );
    // Slot 0 sounds (the engine's meter keyed −id): the glow is on pad 4; pad 0 (slot 4) rests in controlFill.
    rig.rec.0.borrow_mut().meters = vec![ModuleMeter {
        key: ModuleKey {
            channel: 1,
            instance_id: -(id as i32),
        },
        value: MeterSnapshot {
            peak_left: 1.0,
            ..MeterSnapshot::default()
        },
        analysis: vec![],
    }];
    rig.frame(vec![]);
    let theme = &rig.app.theme;
    let control = theme.colors.control_fill.to_color32();
    let channel = crate::theme::channel_color(rig.app.song.channels[1].hue, theme).to_color32();
    let g = crate::ui::drums::meter_level(1.0).powf(1.6667);
    assert_eq!(glow(&rig, 4), control.lerp_to_gamma(channel, g));
    assert_eq!(glow(&rig, 0), control);
}

#[test]
fn a_pad_hit_during_the_count_in_records_nothing_and_starts_no_take() {
    // The take starts when the count-in ends: a hit during it neither records nor sets the take's start.
    let mut rig = rec_rig(true);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.app.audio.counting_in());
    rig.click(pad_centre(0));
    assert!(
        rig.take().contains(&"NoteOn 1 48 127".to_owned()),
        "it plays"
    );
    assert_eq!(drum_hits(&rig), Vec::<Vec<_>>::new());
    // Four beats at the new song's 120 BPM (48 kHz): the count-in ends with the playhead at beat 5, where the take
    // starts.
    rig.render(96_000);
    rig.rec.0.borrow_mut().tick = 5 * BEAT;
    rig.frame(vec![]);
    assert!(!rig.app.audio.counting_in());
    rig.rec.0.borrow_mut().tick = 9 * BEAT;
    rig.click(pad_centre(1));
    // Its clip is at the take's start rounded down (beat 4), not at the count-in's beat 0.
    assert_eq!(drum_hits(&rig), [vec![(4 * PPQ, 5 * PPQ, 1, 0.01)]]);
}

// ---- Live note recording ----

/// A recorded note: (clip start, pos, key, velocity, length in beats).
type Note = (u32, u32, u8, u8, f64);

/// Channel `channel`'s notes lanes (kind 0), by lane: their notes.
fn notes(rig: &Rig, channel: usize) -> Vec<Vec<Note>> {
    let lanes = rig.app.song.channels[channel].lanes.iter();
    lanes
        .filter(|l| l.kind == LaneKind::Notes)
        .map(|l| {
            let notes = l.clips.iter().flat_map(|c| {
                c.events.iter().filter_map(|e| match *e {
                    crate::song::Event::Note {
                        pos,
                        len_beats,
                        key,
                        velocity,
                        ..
                    } => Some((c.start_tick, pos, key, velocity, len_beats)),
                    _ => None,
                })
            });
            notes.collect()
        })
        .collect()
}

/// [`rec_rig`] with the MiniSynth (2) current and the playhead at `tick`; the log is empty.
fn note_rig(count_in: bool, tick: u64) -> Rig {
    let mut rig = rec_rig(count_in);
    rig.app.selected_channel = 2;
    rig.rec.0.borrow_mut().tick = tick;
    rig.frame(vec![]);
    rig.take();
    rig
}

impl Rig {
    /// The output reports the transport at `tick`.
    fn at_tick(&mut self, tick: u64) {
        self.rec.0.borrow_mut().tick = tick;
    }
}

#[test]
fn an_armed_touched_key_starts_playback_and_records_its_note_as_held_in_one_undo_step() {
    let mut rig = note_rig(false, 5 * BEAT + 3);
    let undos = rig.undo_steps();
    rig.click(REC);
    rig.app.files.saved();
    rig.press(E4);
    // The key sounds first; the press starts the transport, and the take starts with the key held.
    rig.frame(vec![]);
    assert!(rig.app.main_screen.playing, "the press started playback");
    // The note held sets the song-changed flag before the take ends.
    assert!(rig.app.files.modified());
    let log = rig.take();
    assert!(log.contains(&"NoteOn 2 52 108".to_owned()), "{log:?}");
    assert!(log.contains(&"Play".to_owned()), "{log:?}");
    rig.at_tick(6 * BEAT + 3 + BEAT / 2);
    rig.release(E4);
    // Clip at floor(5/4)·4 = beat 4 on the synth's own notes lane; no quantize; 1.5 beats held.
    let [lane] = &notes(&rig, 2)[..] else {
        panic!("{:?}", notes(&rig, 2))
    };
    let [(start, pos, key, velocity, len)] = lane[..] else {
        panic!("{lane:?}")
    };
    assert_eq!((start, pos, key, len), (4 * PPQ, PPQ + 3, 52, 1.5));
    assert!(velocity > 200, "the keyboard's own velocity: {velocity}");
    rig.click(PLAY);
    assert_eq!(rig.undo_steps(), undos + 1, "one take, one undo step");
}

#[test]
fn typed_keys_record_at_07_and_nothing_records_with_rec_off_stopped_or_counting_in() {
    use egui::Key;
    // REC off, playing: the key only plays.
    let mut rig = note_rig(true, 0);
    rig.click(PLAY);
    rig.type_key(Key::Z, true);
    rig.type_key(Key::Z, false);
    rig.click(PLAY);
    assert_eq!(notes(&rig, 2), [Vec::new()]);
    // Armed and counting in: it sounds, records nothing; still held when the count-in ends, it records from the
    // take's start.
    rig.click(REC);
    rig.click(PLAY);
    assert!(rig.app.audio.counting_in());
    rig.type_key(Key::X, true);
    rig.type_key(Key::X, false);
    rig.type_key(Key::Z, true);
    assert!(rig.take().contains(&"NoteOn 2 48 89".to_owned()));
    assert_eq!(notes(&rig, 2), [Vec::new()]);
    rig.render(96_000);
    rig.at_tick(4 * BEAT);
    rig.frame(vec![]);
    assert!(!rig.app.audio.counting_in());
    rig.at_tick(5 * BEAT);
    rig.type_key(Key::Z, false);
    // 255·0.7 = 178.5: 178.
    assert_eq!(notes(&rig, 2), [vec![(4 * PPQ, 0, 48, 178, 1.0)]]);
}

#[test]
fn notes_record_at_the_song_position_with_no_latency_compensation() {
    // A note records at the engine's song position when it is handled. Neither the calibration nor the
    // buffer size moves it, though they move an audio take of the same pass earlier.
    let mut rig = note_rig(false, 5 * BEAT + 3);
    rig.app.latency = Some(480);
    rig.app.settings.latency_compensation = None;
    rig.rec.0.borrow_mut().buffer_frames = 256;
    rig.click(REC);
    rig.click(PLAY);
    rig.at_tick(5 * BEAT + 40);
    rig.type_key(egui::Key::Z, true);
    rig.at_tick(6 * BEAT + 40);
    rig.type_key(egui::Key::Z, false);
    assert_eq!(notes(&rig, 2), [vec![(4 * PPQ, PPQ + 40, 48, 178, 1.0)]]);
}

#[test]
fn a_loop_wrap_closes_the_held_note_and_starts_a_new_take_in_a_new_clip_each_one_undo_step() {
    let mut rig = note_rig(false, 0);
    let undos = rig.undo_steps();
    rig.click(REC);
    rig.click(PLAY);
    rig.at_tick(BEAT);
    rig.press(E4);
    rig.at_tick(3 * BEAT);
    rig.frame(vec![]);
    // The loop wraps: the note closes where the take last was (beat 3), and the key, still held, starts the next
    // take's note at its start. Its clip's start (beat 0) is covered by the first take's: a new lane.
    rig.at_tick(BEAT / 2);
    rig.frame(vec![]);
    let log = rig.take();
    assert!(
        log.contains(&"ReplaceSong adopting no-resync".to_owned()),
        "the take plays from now on: {log:?}"
    );
    rig.at_tick(BEAT);
    rig.release(E4);
    let v = notes(&rig, 2)[0][0].3;
    let held = |pos, len| vec![(0, pos, 52, v, len)];
    assert_eq!(notes(&rig, 2), [held(PPQ, 2.0), held(PPQ / 2, 0.5)]);
    // Overdub: the next take keeps every note there.
    rig.at_tick(2 * BEAT);
    rig.frame(vec![]);
    rig.at_tick(0);
    rig.frame(vec![]);
    rig.click(PLAY);
    assert_eq!(notes(&rig, 2), [held(PPQ, 2.0), held(PPQ / 2, 0.5)]);
    assert_eq!(rig.undo_steps(), undos + 2, "an empty take is no undo step");
}

#[test]
fn a_key_held_at_stop_keeps_length_0_and_its_release_records_nothing() {
    // A typed key: the one pointer cannot hold a piano key while it taps Play.
    let mut rig = note_rig(false, 0);
    rig.click(REC);
    rig.click(PLAY);
    rig.at_tick(BEAT);
    rig.type_key(egui::Key::Z, true);
    rig.at_tick(2 * BEAT);
    rig.click(PLAY);
    assert!(!rig.app.main_screen.armed);
    rig.at_tick(3 * BEAT);
    rig.type_key(egui::Key::Z, false);
    let [lane] = &notes(&rig, 2)[..] else {
        panic!()
    };
    assert_eq!(lane.iter().map(|n| n.4).collect::<Vec<_>>(), [0.0]);
    // The engine plays the song with it (a length-0 note plays nothing): the stop's rebuild went out.
    assert_eq!(rig.app.audio.built_song(), Some(&rig.app.song));
}

#[test]
fn the_take_shows_as_a_live_bar_from_its_clip_start_to_the_playhead_until_it_ends() {
    use crate::ui::clips::LiveTake;
    let mut rig = note_rig(false, 5 * BEAT);
    rig.click(REC);
    rig.click(PLAY);
    assert_eq!(rig.app.main_screen.takes, [], "no note, no clip yet");
    rig.press(E4);
    rig.at_tick(7 * BEAT);
    rig.frame(vec![]);
    let id = rig.app.song.channels[2].lanes[0].clips[0].id;
    let bar = LiveTake {
        channel: 2,
        lane: 0,
        start: (4 * BEAT) as f64,
        end: (7 * BEAT) as f64,
        clip: Some(id),
    };
    assert_eq!(rig.app.main_screen.takes, [bar]);
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(rig.app.main_screen.takes, []);
}

#[test]
fn a_channel_change_while_recording_closes_the_old_takes_notes_and_records_on_the_new_channel() {
    use egui::Key;
    let mut rig = note_rig(false, 0);
    rig.click(REC);
    rig.click(PLAY);
    rig.at_tick(BEAT);
    rig.type_key(Key::Z, true);
    // The Drums channel becomes current: the typed key, which stays on the synth, closes there now.
    rig.at_tick(2 * BEAT);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    assert!(rig.app.main_screen.armed && rig.app.main_screen.playing);
    rig.at_tick(3 * BEAT);
    rig.type_key(Key::Z, false);
    assert_eq!(notes(&rig, 2), [vec![(0, PPQ, 48, 178, 1.0)]]);
    // On the drums, typed key 48 plays the kit's slot 0: a hit on a new flag-1 lane, at least 0.01 beat long.
    rig.type_key(Key::Z, true);
    rig.at_tick(3 * BEAT + 1);
    rig.type_key(Key::Z, false);
    let ch = &rig.app.song.channels[1];
    let lane = ch.lanes.last().unwrap();
    assert_eq!(lane.grid, crate::song::DrumGrid::Notes);
    let hit = crate::song::Event::Drum {
        pos: 3 * PPQ,
        len_beats: 0.01,
        pad: 0,
        velocity: 178,
        pan: crate::song::Event::DEFAULT_PAN,
        value: crate::song::Event::DEFAULT_PITCH,
        repeat: 0,
    };
    // The drum take's clip starts from the take's start (beat 2), rounded down to 4 beats: beat 0.
    assert_eq!(lane.clips[0].start_tick, 0);
    assert_eq!(lane.clips[0].events, [hit]);
}

#[test]
fn a_moved_channel_or_a_new_song_leaves_no_phantom_held_note_for_the_next_take() {
    use egui::Key;
    let mut rig = note_rig(false, 0);
    rig.app
        .add_channel(ChannelKind::Synth(SynthKind::MiniSynth), 0);
    rig.app.selected_channel = 2;
    rig.frame(vec![]);
    rig.type_key(Key::Z, true);
    // The drums go: the synth holding the key is channel 1 now, the other synth channel 2.
    rig.app.song.channels.remove(1);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.selected_channel = 2;
    rig.click(REC);
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(
        notes(&rig, 2),
        [Vec::new()],
        "no phantom on the other synth"
    );
    rig.click(PLAY);
    // A new song with a synth at 1: nothing held there either.
    let mut song = Song::default();
    song.add_channel(ChannelKind::Synth(SynthKind::MiniSynth));
    rig.app.song = song;
    rig.app.song_edited(SongEdit::Replaced, 0);
    rig.app.selected_channel = 1;
    rig.frame(vec![]);
    rig.click(REC);
    rig.click(PLAY);
    rig.frame(vec![]);
    assert_eq!(notes(&rig, 1), [Vec::new()]);
}

/// The rects the last frame filled inside `area` whose (rect, fill) `keep` takes, groups flattened.
fn fills_in(
    rig: &Rig,
    area: egui::Rect,
    keep: impl Fn(egui::Rect, egui::Color32) -> bool,
) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    (rig.out.shapes.iter()).for_each(|s| walk(&s.shape, &mut all));
    (all.into_iter())
        .filter(|&(r, fill)| area.contains_rect(r) && keep(r, fill))
        .map(|(r, _)| r)
        .collect()
}

#[test]
fn a_held_note_shows_live_in_the_playlist_and_the_open_piano_roll_as_the_take_then_stores_it() {
    // Honeydew's own (issue #26). The roll is open on the synth's (2) empty one-bar clip at 0, which the take replaces:
    // it stays open, on the take's clip.
    let mut rig = Rig::with_drums_and_synth();
    rig.app.settings.count_in = false;
    rig.click(OUTSIDE);
    rig.edit(2);
    rig.frame(vec![]);
    rig.app.main_screen.split.target = 0.6;
    rig.slide_split();
    rig.at_tick(BEAT);
    rig.click(REC);
    rig.click(PLAY);
    rig.press(E4);
    let ink =
        crate::theme::channel_color(rig.app.song.channels[2].hue, &crate::theme::theme_by_id(0));
    // The note as drawn: in the playlist, the 2 px bar on the synth's row (the third); in the roll, its cell.
    let drawn = |rig: &mut Rig, tick| {
        rig.at_tick(tick);
        rig.frame(vec![]);
        let row = egui::Rect::from_x_y_ranges(50.0..=1170.0, RULER + 4.0 * U..=RULER + 6.0 * U);
        let grid = rig
            .ctx
            .read_response(egui::Id::new("piano_roll").with("grid"));
        let grid = grid.expect("the roll").rect;
        let bar = fills_in(rig, row, |r, _| r.height() == 2.0);
        let cell = fills_in(rig, grid, |r, fill| {
            fill == ink.to_color32() && r.height() > 10.0
        });
        (bar, cell)
    };
    let (bar, cell) = drawn(&mut rig, 2 * BEAT);
    let [bar] = bar[..] else { panic!("{bar:?}") };
    let [cell] = cell[..] else { panic!("{cell:?}") };
    let clip = rig.app.song.channels[2].lanes[0].clips[0].id;
    assert_eq!(rig.app.nav.view, View::PianoRoll(2));
    assert_eq!(rig.app.slots.piano_roll.rolls[&2].clip_id, Some(clip));
    // A beat later the held note reaches a beat further, from the same note-on.
    let (grown, longer) = drawn(&mut rig, 3 * BEAT);
    let ([grown], [longer]) = (&grown[..], &longer[..]) else {
        panic!("{grown:?} {longer:?}")
    };
    let (grown, longer) = (*grown, *longer);
    assert_eq!((grown.left(), longer.left()), (bar.left(), cell.left()));
    assert!(
        (grown.width() - bar.width() - 38.4).abs() < 1e-3,
        "{bar:?} {grown:?}"
    );
    assert!(longer.width() > cell.width(), "{cell:?} {longer:?}");
    // Let go there, and the take ends: the clip is what showed, the roll still on it.
    rig.release(E4);
    rig.click(PLAY);
    let (stored, kept) = drawn(&mut rig, 3 * BEAT);
    assert_eq!((stored, kept), (vec![grown], vec![longer]));
    let v = notes(&rig, 2)[0][0].3;
    assert_eq!(notes(&rig, 2), [vec![(0, PPQ, 52, v, 2.0)]]);
    assert_eq!(
        rig.app.song.channels[2].lanes[0].clips[0].visible_len_beats,
        3.0
    );
    assert_eq!(rig.app.slots.piano_roll.rolls[&2].clip_id, Some(clip));
}
