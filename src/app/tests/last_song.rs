//! Reopening the last song (#58): a start opens the song file open at the last exit again.

use super::*;
use crate::project_files::content::tests::{Temp, put};
use crate::ui::main_menu::{NativeLibrary, SONGS_FOLDER};

/// A start with the settings `spy` holds, and its songs on disk in `user`'s songs folder.
fn start(spy: &Spy, user: &std::path::Path) -> HoneydewApp {
    let mut app = HoneydewApp::with_storage(Some(Box::new(spy.clone())));
    app.files = without_factory(ProjectFiles::on_disk(user));
    app.files.codec = synthetic_codec();
    app.files.library = Box::new(NativeLibrary::at(user.join(SONGS_FOLDER)));
    app
}

#[test]
fn a_start_opens_the_song_saved_before_the_last_exit_again() {
    let user = Temp::new("last-song");
    let beat = user.0.join(SONGS_FOLDER).join("Beat.flm");
    put(&beat, &synthetic_file(1, FORMAT_VERSION, ""));
    let spy = Spy::default();
    // The first start: the song opens from SONGS, gets a channel and is saved; then the app closes.
    let mut rig = Rig::new();
    rig.app = start(&spy, &user.0);
    rig.app.song = rig.app.files.replace(Replace::Open(beat)).unwrap();
    rig.app.song_replaced(&rig.ctx);
    rig.app.add_default_drums(0);
    rig.app.files.save(&rig.app.song);
    rig.frame(vec![]);
    assert!(!rig.app.files.modified(), "saved");
    // The next start opens it again, as saved.
    let ctx = egui::Context::default();
    let mut app = start(&spy, &user.0);
    app.restore_last_song(&ctx);
    assert_eq!((&app.song.title[..], app.song.channels.len()), ("Beat", 3));
    assert!(!app.files.modified());
    // That start ended while it opened the song (`interruptedWhenLoadingSong` is still set): the next opens none.
    let mut app = start(&spy, &user.0);
    app.restore_last_song(&ctx);
    assert_eq!(app.song.channels.len(), 1, "the new song");
}
