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
