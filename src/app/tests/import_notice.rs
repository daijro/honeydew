//! The first-import notice through the app: its `Import` is kept in the
//! settings, so a later start imports what waits in the Original Content folder without asking.

use super::*;
use crate::project_files::content::tests::{Temp, put};
use crate::project_files::{CONTENT_FOLDER, IMPORT_FAILED, NOTICE_TAG};

/// A start with the settings `spy` keeps and the files on disk in `user`, which look for archives as the app's do.
fn start(spy: &Spy, user: &std::path::Path) -> Rig {
    let mut rig = Rig::new();
    let settings = Settings::load(spy);
    rig.app.settings_saved = settings.clone();
    rig.app.settings = settings;
    rig.app.storage = Some(Box::new(spy.clone()));
    rig.app.files = ProjectFiles::on_disk(user);
    rig.app.files.import_archives();
    rig
}

/// The tag and first line of the message shown after a few frames (the import's worker has time to finish).
fn message_after_frames(rig: &mut Rig) -> Option<(&'static str, String)> {
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        rig.frame(vec![]);
    }
    rig.overlays().iter().find_map(|o| match o {
        Overlay::Message { tag, lines, .. } => Some((*tag, lines[0].clone())),
        _ => None,
    })
}

/// The message's choices, its rows that take a tap: `Import`, then `Not now`.
fn choices(rig: &Rig) -> [egui::Id; 2] {
    let row = |r: usize| egui::Id::new("message").with((0_usize, r));
    let rows: Vec<_> = (0..100)
        .map(row)
        .filter(|&id| rig.ctx.read_response(id).is_some())
        .collect();
    rows.try_into().expect("two choices")
}

#[test]
fn import_on_the_notice_is_kept_and_a_later_start_does_not_ask() {
    let tmp = Temp::new("notice-app");
    // An archive that fails: the import's own message shows that it ran, and it waits again at the next start.
    put(
        &tmp.0.join(CONTENT_FOLDER).join("Gen.obb"),
        b"not a container",
    );
    let spy = Spy::default();
    // `Not now`: nothing imports, nothing is kept.
    let mut rig = start(&spy, &tmp.0);
    assert_eq!(message_after_frames(&mut rig).unwrap().0, NOTICE_TAG);
    rig.click_id(choices(&rig)[1]);
    assert_eq!(message_after_frames(&mut rig), None);
    assert!(!Settings::load(&spy).import_notice_accepted);
    // The next start asks again; `Import` imports and is kept.
    let mut rig = start(&spy, &tmp.0);
    assert_eq!(message_after_frames(&mut rig).unwrap().0, NOTICE_TAG);
    rig.click_id(choices(&rig)[0]);
    let failed = Some((MESSAGE_TAG, IMPORT_FAILED.to_owned()));
    assert_eq!(message_after_frames(&mut rig), failed);
    assert!(Settings::load(&spy).import_notice_accepted);
    // A later start imports without the notice.
    let mut rig = start(&spy, &tmp.0);
    let failed = Some((MESSAGE_TAG, IMPORT_FAILED.to_owned()));
    assert_eq!(message_after_frames(&mut rig), failed);
}
