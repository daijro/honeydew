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
/// The first-open question: download the sound library now, or later from UPDATES / Settings.
const SOUNDS_ASK_TAG: &str = "update_sounds_ask";

impl HoneydewApp {
    pub(super) fn updates_at_start(&mut self) {
        update::allow_mobile_data(self.settings.sounds_mobile_data);
        if self.settings.check_updates {
            update::check(true);
        }
        // The base APK ships no sounds: the first open asks whether to download them now, once. They never start
        // on their own; UPDATES and Settings `Install sound library…` start (and pause/continue) them by hand.
        if cfg!(target_os = "android")
            && !self.settings.sounds_asked
            && install::needed(self.files.factory.bundle.as_deref())
        {
            self.settings.sounds_asked = true;
            self.nav.overlays.push(Overlay::Message {
                tag: SOUNDS_ASK_TAG,
                lines: vec![
                    "This build has no sounds inside.".into(),
                    "Download the sound library now (about 740 MB, over Wi-Fi unless you allow mobile data in UPDATES)?"
                        .into(),
                ],
                confirm: Some(("Download".into(), "Not now".into())),
                extra: None,
            });
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
        match self.nav.take_answer(SOUNDS_ASK_TAG) {
            Some(true) => self.download_sounds(),
            Some(false) => {
                self.navigate(Nav::Open(Overlay::Message {
                    tag: SOUNDS_TAG,
                    lines: vec![
                        "No sounds downloaded.".into(),
                        "Download them any time: Main menu > UPDATES > Download sound library, or Main menu > SETTINGS > Install sound library….".into(),
                    ],
                    confirm: None,
                    extra: None,
                }));
            }
            None => {}
        }
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
