//! Audio files dropped on the app: kept in `My Samples`, each an audio clip where it lands, one undo step a drop.

use std::path::Path;

use super::*;
use crate::flm::chunk::{ContentPath, ContentRoot};
use crate::history::Step;
use crate::song::PPQ;
use crate::ui::views::preset_picker::join_workers;

/// A frame in which `files` drop with the pointer at `at`, then the frames their decode takes to land.
fn drop_at(rig: &mut Rig, at: Pos2, files: &[&str]) {
    // Two seconds of silence: 4 beats at 120 BPM.
    let wav = crate::sample_cache::tests::wav(&[0; 88_200]);
    let dropped_files = (files.iter())
        .map(|name| egui::DroppedFile {
            name: (*name).into(),
            bytes: Some(wav.clone().into()),
            ..Default::default()
        })
        .collect();
    rig.time += 0.1;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1170.0, 540.0))),
        events: vec![egui::Event::PointerMoved(at)],
        dropped_files,
        time: Some(rig.time),
        ..Default::default()
    };
    let Rig { ctx, app, out, .. } = rig;
    *out = ctx.run(input, |ctx| app.frame(ctx, |_, _| None));
    join_workers();
    rig.frame(vec![]);
}

/// The sample file of `channel`'s first track's clips, by start tick.
fn clips(rig: &Rig, channel: usize) -> Vec<(u32, String)> {
    let lane = &rig.app.song.channels[channel].lanes[0];
    (lane.clips.iter())
        .map(|c| {
            let slot = c.file.sample.as_ref().expect("a sample");
            let Some(ContentPath::Shorthand {
                root: ContentRoot::User,
                relative,
            }) = &slot.file.path
            else {
                panic!("{:?}", slot.file.path);
            };
            (c.start_tick, relative.clone())
        })
        .collect()
}

#[test]
fn dropped_audio_files_become_clips_on_the_audio_track_or_new_channels_one_undo_step_a_drop() {
    let mut rig = Rig::new();
    rig.app.add_bare_channel(ChannelKind::AudioClip, 0);
    rig.frame(vec![]);
    let steps = rig.app.history.steps();
    // On the audio track (row 1): copies in `My Samples`, the clips one after another from the snapped beat.
    drop_at(&mut rig, pos2(600.0, header(1).y), &["a.wav", "b.wav"]);
    let made = clips(&rig, 1);
    let [(start, a), (next, b)] = &made[..] else {
        panic!("{made:?}");
    };
    assert_eq!(
        (a.as_str(), b.as_str()),
        ("My Samples/a.wav", "My Samples/b.wav")
    );
    assert!(*start > 0 && start % PPQ == 0, "{start}");
    assert_eq!(*next, start + 4 * PPQ);
    assert!(
        rig.app
            .files
            .user_file_exists(Path::new("My Samples/b.wav"))
    );
    assert_eq!(rig.app.history.steps(), steps + 1);
    // Off the playlist (over the transport bar): a new Audio channel at the end, its clip at the playhead; the same
    // name again is kept under a free one.
    drop_at(&mut rig, PLAY, &["a.wav"]);
    assert_eq!(rig.app.song.channels.len(), 3);
    assert_eq!(rig.app.song.channels[2].kind, ChannelKind::AudioClip);
    assert_eq!(clips(&rig, 2), [(0, "My Samples/a (1).wav".to_owned())]);
    assert_eq!(rig.app.history.steps(), steps + 2);
    // One undo a drop.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, Step::Undo, 0);
    assert_eq!(rig.app.song.channels.len(), 2);
    rig.app.step(&ctx, Step::Undo, 0);
    assert!(rig.app.song.channels[1].lanes[0].clips.is_empty());
}

/// #167 (a Honeydew change): Tesla's Convert to score makes a new clip at the playhead, on the selected channel when it
/// has a notes track, else on the first channel after it with one, else on a new MiniSynth channel; the clip is current,
/// its loop fitted to the notes, one undo step a send.
#[test]
fn tesla_scores_become_a_new_clip_on_the_selected_or_next_notes_channel_or_a_new_minisynth() {
    use crate::song::{Event, SynthKind};
    use os_plugkit::embed::NoteOut;
    let mut rig = Rig::with_drums_and_synth();
    let note = |start, end, key| NoteOut {
        start,
        end,
        key,
        velocity: 127,
    };
    let notes = [note(4.0, 5.5, 69), note(0.0, 0.5, 60)];
    let steps = rig.app.history.steps();
    // The drums selected: the synth after them takes the clip, at the playhead.
    rig.app.selected_channel = 1;
    rig.app.place_score(&notes, 256);
    let clips = |rig: &Rig, c: usize| rig.app.song.channels[c].lanes[0].clips.clone();
    let [clip] = &clips(&rig, 2)[..] else {
        panic!("{:?}", clips(&rig, 2));
    };
    assert_eq!((clip.start_tick, clip.loop_len_beats), (256, 8.0));
    let note = |pos, len_beats, key| Event::Note {
        pos,
        len_beats,
        key,
        velocity: 254,
        pan: Event::DEFAULT_PAN,
        slide: false,
    };
    assert_eq!(clip.events, [note(0, 0.5, 60), note(512, 1.5, 69)]);
    assert_eq!(rig.app.selected_channel, 2);
    let current = rig.current_clip().map(|r| (r.channel, r.lane, r.id));
    assert_eq!(current, Some((2, 0, clip.id)));
    assert_eq!(rig.app.history.steps(), steps + 1);
    // The synth selected: its own track again, after that clip.
    rig.app.place_score(&notes, 256 + 1024);
    assert_eq!(clips(&rig, 2).len(), 2);
    // An audio channel past the last notes channel: a new MiniSynth channel at the end.
    rig.app.add_bare_channel(ChannelKind::AudioClip, 0);
    rig.app.place_score(&notes, 0);
    let synth = ChannelKind::Synth(SynthKind::MiniSynth);
    assert_eq!(rig.app.song.channels.len(), 5);
    assert_eq!(rig.app.song.channels[4].kind, synth);
    assert_eq!(clips(&rig, 4).len(), 1);
    assert_eq!(rig.app.history.steps(), steps + 4);
}

/// #167: a score never lands over a clip: it takes the chosen channel's first notes track with room under its span at
/// the playhead, else a new track of that channel, still one undo step.
#[test]
fn a_tesla_score_takes_the_channels_next_free_track_at_the_playhead() {
    use crate::song::LaneKind::Notes;
    use os_plugkit::embed::NoteOut;
    let mut rig = Rig::with_drums_and_synth();
    rig.app.song.channels[2].lanes[0].add_clip(0, 4.0);
    rig.app.song_edited(SongEdit::Final, 0);
    rig.app.selected_channel = 2;
    let steps = rig.app.history.steps();
    // One bar of notes: a 4-beat clip.
    let notes = [NoteOut {
        start: 0.0,
        end: 1.0,
        key: 60,
        velocity: 100,
    }];
    let tracks = |rig: &Rig| {
        (rig.app.song.channels[2].lanes.iter())
            .map(|l| (l.kind, l.clips.iter().map(|c| c.start_tick).collect()))
            .collect::<Vec<(_, Vec<u32>)>>()
    };
    // Over the main track's clip: a new track.
    rig.app.place_score(&notes, 256);
    assert_eq!(tracks(&rig), [(Notes, vec![0]), (Notes, vec![256])]);
    let current = rig.current_clip().map(|r| (r.channel, r.lane));
    assert_eq!(current, Some((2, 1)));
    // Where that clip has ended, the main track has room again.
    rig.app.place_score(&notes, 512);
    assert_eq!(tracks(&rig), [(Notes, vec![0, 512]), (Notes, vec![256])]);
    assert_eq!(rig.app.history.steps(), steps + 2);
}
