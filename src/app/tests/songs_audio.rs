//! SONGS' audio files through the whole app (#106): a tap previews one on the sample strip, `Send to playlist` makes
//! clips of the selected ones.

use super::songs_archive::{Home, row_at};
use super::*;
use crate::song::PPQ;
use crate::ui::main_menu::NativeLibrary;
use crate::ui::views::preset_picker::join_workers;

/// A rig with SONGS open on `My Samples` of a user folder on disk that holds `kick.wav`, 2 s (rig frames are 0.1 s apart); the file's path.
fn rig_in_my_samples(home: &Home) -> (Rig, PathBuf) {
    let user = home.0.join("user");
    let samples = user.join("My Samples");
    std::fs::create_dir_all(user.join("My Songs")).unwrap();
    std::fs::create_dir_all(&samples).unwrap();
    let kick = samples.join("kick.wav");
    std::fs::write(&kick, crate::sample_cache::tests::wav(&[0; 88200])).unwrap();
    let mut rig = Rig::new();
    rig.app.files = without_factory(ProjectFiles::on_disk(&user));
    rig.app.files.library = Box::new(NativeLibrary::at(user.join("My Songs")));
    rig.open_menu(MenuTab::Songs);
    // `Back` to the user folder, then into `My Samples`.
    rig.click_id(songs_id(("side", 0_usize)));
    rig.frame(vec![]);
    rig.click_id(row_at(&samples));
    rig.frame(vec![]);
    (rig, kick)
}

#[test]
fn a_tap_on_an_audio_file_in_songs_previews_it_and_a_second_tap_stops_it() {
    let home = Home::new("songs-preview");
    let (mut rig, kick) = rig_in_my_samples(&home);
    rig.take();
    rig.click_id(row_at(&kick));
    join_workers();
    rig.frame(vec![]);
    rig.frame(vec![]);
    let sent = rig.take();
    assert!(
        sent.contains(&"Preview 88200 frames from 0".to_owned()),
        "{sent:?}"
    );
    // Tapped again it is no longer selected: the preview stops.
    rig.click_id(row_at(&kick));
    rig.frame(vec![]);
    let sent = rig.take();
    assert!(sent.contains(&"Preview stop".to_owned()), "{sent:?}");
}

#[test]
fn send_to_playlist_puts_the_selected_audio_on_a_new_track_at_the_playhead_in_one_undo_step() {
    let home = Home::new("songs-send");
    let (mut rig, kick) = rig_in_my_samples(&home);
    rig.rec.0.borrow_mut().tick = 2 * u64::from(PPQ);
    let (channels, steps) = (rig.app.song.channels.len(), rig.app.history.steps());
    rig.click_id(row_at(&kick));
    rig.frame(vec![]);
    // Select mode's bar: `Select all`, `Create folder`, then `Send to playlist`.
    rig.click_id(songs_id(("bar", 2_usize)));
    join_workers();
    rig.frame(vec![]);
    rig.frame(vec![]);
    // The menu closes on it; a new Audio channel holds the clip, at the playhead.
    assert_eq!(rig.overlays(), []);
    let song = &rig.app.song;
    assert_eq!(song.channels.len(), channels + 1);
    let new = song.channels.last().unwrap();
    assert_eq!(new.kind, ChannelKind::AudioClip);
    let starts: Vec<u32> = new.lanes[0].clips.iter().map(|c| c.start_tick).collect();
    assert_eq!(starts, [2 * PPQ]);
    assert_eq!(rig.app.history.steps(), steps + 1);
}
