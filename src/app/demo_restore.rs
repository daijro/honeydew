//! SETTINGS `Restore demo songs` in the app (under Advanced): no question; the loading screen [`RESTORING_DEMO_SONGS`]
//! (the export's `Overlay::Busy`) comes up, the copy runs on the frame after it shows
//! ([`ProjectFiles::restore_demo_songs`](crate::project_files::ProjectFiles::restore_demo_songs)), the screen holds
//! [`HOLD_SECS`] after it, then closes on the flash [`RESTORED`]. A failed copy closes it at once and shows its
//! message. The first launch's copy is the app's start hook, with no screen or flash (nothing raises
//! `Demo songs extracted`).

use eframe::egui;

use super::HoneydewApp;
use crate::ui::views::{Nav, Overlay};

/// The loading screen's title.
pub(super) const RESTORING_DEMO_SONGS: &str = "Restoring demo songs...";
/// The flash once it is done.
pub(super) const RESTORED: &str = "Restored";
/// How long the screen stays up after the copy, in seconds.
const HOLD_SECS: f64 = 1.0;
/// The failure message's tag.
const FAILED_TAG: &str = "restore_demo_songs";

/// Where a restore is.
#[derive(Clone, Copy, Debug)]
pub(super) enum DemoRestore {
    /// The screen is up; the copy runs next frame.
    Copy,
    /// Copied at this time (egui seconds); the screen holds.
    Held(f64),
}

impl HoneydewApp {
    /// After the screens, each frame: starts a restore SETTINGS asked for, then runs it on.
    pub(super) fn demo_restore(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.files.restore_demos) {
            let title = RESTORING_DEMO_SONGS.into();
            self.navigate(Nav::Open(Overlay::Busy { title }));
            self.demo_restore = Some(DemoRestore::Copy);
            ctx.request_repaint();
            return;
        }
        let Some(stage) = self.demo_restore else {
            return;
        };
        match stage {
            DemoRestore::Copy => match self.files.restore_demo_songs() {
                Ok(()) => {
                    self.settings.demo_songs_restored = true;
                    self.demo_restore = Some(DemoRestore::Held(self.now));
                    ctx.request_repaint_after_secs(HOLD_SECS as f32);
                }
                Err(err) => {
                    self.demo_restore_done();
                    let lines = vec![format!("{}: {err}", crate::content::DEMO_FOLDER)];
                    self.navigate(Nav::Open(Overlay::Message {
                        tag: FAILED_TAG,
                        lines,
                        confirm: None,
                        extra: None,
                    }));
                }
            },
            DemoRestore::Held(since) if self.now - since < HOLD_SECS => {
                ctx.request_repaint_after_secs((HOLD_SECS - (self.now - since)) as f32);
            }
            DemoRestore::Held(_) => {
                self.demo_restore_done();
                self.files.flash(RESTORED);
            }
        }
    }

    /// Closes the loading screen.
    fn demo_restore_done(&mut self) {
        self.demo_restore = None;
        let screen =
            |o: &Overlay| matches!(o, Overlay::Busy { title } if title == RESTORING_DEMO_SONGS);
        self.nav.overlays.retain(|o| !screen(o));
    }
}
