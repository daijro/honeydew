//! The desktop's sound library (`content::install`) in the app. At start, while there is none, or the one found is
//! older than this version's, the question [`sounds_question`] says so, once per shipped sound release; SETTINGS
//! `Install sound library…` asks it too. `ZIP file…` or `Folder…` opens the system's picker, then the sync runs on a worker behind the loading screen,
//! [`INSTALLING`] with its percent and the abort X; an abort leaves the library as it was. Once done, a message says
//! to restart: the factory loads the content once ([`crate::content::factory`]).

use std::time::Duration;

use eframe::egui;

use super::HoneydewApp;
use crate::content::install::{self, Job};
use crate::metrics::Metrics;
use crate::ui::UiCtx;
use crate::ui::views::main_menu::{SOUNDS_TAG, sounds_question};
use crate::ui::views::{self, Answer, Nav, Overlay};

/// The loading screen's title, before the percent.
pub(super) const INSTALLING: &str = "Installing sound library...";
/// The message's tag once it is done.
const DONE_TAG: &str = "sounds_installed";

impl HoneydewApp {
    /// At start, the question while the sound library is missing or older than this version's: once per shipped sound
    /// release (`soundsNoticeSeen`), as some do without the sounds.
    pub(super) fn sounds_at_start(&mut self) {
        let shipped = install::SHIPPED_RELEASE.trim();
        let seen = &self.settings.sounds_notice_seen;
        if !seen.is_empty() && !install::older(seen, shipped) {
            return;
        }
        let why = match &self.files.factory.bundle {
            None => "The sound library is missing.".to_owned(),
            Some(bundle) if install::older(&bundle.index.release, shipped) => format!(
                "The sound library ({}) is older than this version's ({shipped}).",
                bundle.index.release
            ),
            Some(_) => return,
        };
        self.settings.sounds_notice_seen = shipped.into();
        self.navigate(sounds_question(Some(why)));
    }

    /// After the screens, each frame: the question's answer picks the source and starts the install, then its loading
    /// screen shows until it ends.
    pub(super) fn sounds(&mut self, ctx: &egui::Context, m: &Metrics) {
        if self.nav.answer().is_some_and(|(tag, _)| tag == SOUNDS_TAG) {
            let source = match self.nav.take_choice(SOUNDS_TAG) {
                Some(Answer::Extra) => (rfd::FileDialog::new())
                    .add_filter("Sound library", &["zip"])
                    .pick_file(),
                Some(Answer::Yes) => rfd::FileDialog::new().pick_folder(),
                _ => None,
            };
            if let (Some(source), Some(target)) = (source, install::dir()) {
                self.sounds = Some(Job::start(source, target));
            }
        }
        let Some(job) = &self.sounds else {
            return;
        };
        let Some(result) = job.result() else {
            let cx = UiCtx {
                theme: &self.theme,
                m,
            };
            if views::busy_abort(ctx, &cx, &format!("{INSTALLING} {}%", job.percent())) {
                job.cancel();
            }
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let cancelled = job.cancelled();
        self.sounds = None;
        let lines = match result {
            Ok(()) => vec![
                "The sound library is installed.".into(),
                format!("Restart {} to use it.", crate::APP_NAME),
            ],
            Err(_) if cancelled => return,
            Err(why) => vec!["The sound library could not be installed.".into(), why],
        };
        self.navigate(Nav::Open(Overlay::Message {
            tag: DONE_TAG,
            lines,
            confirm: None,
            extra: None,
        }));
    }
}
