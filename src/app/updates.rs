//! In-app updates and the sound library's download in the app (#66, [`crate::update`]). At start: the check while
//! `Check for updates automatically` is on, and on Android the sound library's download where one is to be fetched
//! (the base APK; the desktop's is `app/sounds.rs`'s). Each frame: news of a newer release, once per version, with
//! `Show` opening UPDATES; and how a sound library download ended. Neither replaces a message that is open.

use super::HoneydewApp;
use crate::content::install;
use crate::ui::views::{MenuTab, Nav, Overlay};
use crate::update;

/// The news of a newer release, and the sound library's download's end.
const NEWS_TAG: &str = "update_news";
const SOUNDS_TAG: &str = "update_sounds";

impl HoneydewApp {
    pub(super) fn updates_at_start(&mut self) {
        update::allow_mobile_data(self.settings.sounds_mobile_data);
        if self.settings.check_updates {
            update::check(true);
        }
        if cfg!(target_os = "android") && install::needed(self.files.factory.bundle.as_deref()) {
            self.download_sounds();
        }
    }

    /// Downloads the sound library, with a flash that says where to follow it.
    pub(super) fn download_sounds(&mut self) {
        update::download_sounds();
        (self.files)
            .flash("Downloading the sound library...\nMain menu > UPDATES shows how far it is.");
    }

    pub(super) fn updates(&mut self) {
        update::allow_mobile_data(self.settings.sounds_mobile_data);
        if self.nav.take_answer(NEWS_TAG) == Some(true) {
            self.navigate(Nav::Open(Overlay::MainMenu(MenuTab::Updates)));
        }
        let message = |o: &Overlay| matches!(o, Overlay::Message { .. });
        if self.nav.overlays.iter().any(message) {
            return;
        }
        let missing = self.files.factory.bundle.is_none();
        let lines = match update::take_sounds_end() {
            Some(Ok(())) => vec![
                "The sound library is downloaded.".into(),
                format!("Restart {} to use it.", crate::APP_NAME),
            ],
            // An update of the library fails quietly (UPDATES tells why); without one, the user is told.
            Some(Err(why)) if missing => {
                vec!["The sound library could not be downloaded.".into(), why]
            }
            _ => match self.settings.check_updates.then(update::announce).flatten() {
                Some(version) => {
                    let line = format!("{} {version} is available.", crate::APP_NAME);
                    let confirm = Some(("Show".into(), "Later".into()));
                    let tag = NEWS_TAG;
                    let extra = None;
                    let lines = vec![line];
                    self.navigate(Nav::Open(Overlay::Message {
                        tag,
                        lines,
                        confirm,
                        extra,
                    }));
                    return;
                }
                None => return,
            },
        };
        let (tag, confirm, extra) = (SOUNDS_TAG, None, None);
        self.navigate(Nav::Open(Overlay::Message {
            tag,
            lines,
            confirm,
            extra,
        }));
    }
}
