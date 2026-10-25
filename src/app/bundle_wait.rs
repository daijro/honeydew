//! A song load on the web waits for the bundled sounds it needs (`content::web`): a loading screen counts their parts
//! in, then the song's audio loads as it does at once elsewhere ([`HoneydewApp::song_audio`]), so nothing decodes twice
//! and no sound still on its way reads as missing. A program that stood in with its lite one loads in full once all its
//! parts are in.

use eframe::egui;

use super::HoneydewApp;
use crate::audio_engine::samples;
use crate::audio_link::Handover;
use crate::content::web;
use crate::sample_cache::{self, SampleCache, SampleKey};
use crate::ui::views::{Nav, Overlay};

/// The loading screen's title while a part is tried again.
pub(super) const RETRYING: &str = "Something went wrong. Retrying...";

/// The loading screen's title with `landed` of `total` parts in.
fn downloading(landed: usize, total: usize) -> String {
    format!("Downloading sounds ({landed}/{total})...")
}

impl HoneydewApp {
    /// A new song (`SongEdit::Replaced`, its cache new): on the web, the bundled sounds it needs that are not in memory
    /// start downloading behind the loading screen, and its audio waits for them ([`Self::bundle_wait`]). Elsewhere, or
    /// with them all in, it loads now.
    pub(super) fn song_loaded(&mut self) {
        let bundle = self.files.factory.bundle.as_deref();
        let keys = bundle.map(|_| {
            let mut keys = SampleCache::referenced(&self.song);
            if samples::may_stand_in(&self.song) {
                keys.push(SampleKey::File(samples::stand_in()));
            }
            keys
        });
        self.song_download = bundle
            .zip(keys)
            .and_then(|(bundle, keys)| web::song(bundle, &keys));
        match &self.song_download {
            Some(ticket) => {
                let progress = ticket.progress();
                let title = downloading(progress.landed, progress.total);
                self.navigate(Nav::Open(Overlay::Busy { title }));
            }
            None => {
                self.close_busy();
                self.song_audio();
            }
        }
    }

    fn close_busy(&mut self) {
        (self.nav.overlays).retain(|o| !matches!(o, Overlay::Busy { .. }));
    }

    /// A new song's audio: a fresh engine, and whether audio files are missing (once per load).
    pub(super) fn song_audio(&mut self) {
        self.rebuild(0);
        // Looked up here too when no engine was built (no audio yet).
        let read = sample_cache::reader(&self.files);
        self.audio.samples.load_live(&self.song, &self.song, &read);
        self.missing_audio = self.audio.samples.missing().next().is_some();
    }

    /// Each frame: the loader's downloads go on; a song waiting for them shows its count, or [`RETRYING`], until each
    /// is in or given up, then loads; a program its lite one stood in for loads in full.
    pub(super) fn bundle_wait(&mut self, ctx: &egui::Context, tick: u64) {
        let upgraded = web::poll(ctx);
        let factory = self.files.factory;
        if let Some(bundle) = factory.bundle.as_deref() {
            web::trim(bundle, &self.song);
        }
        if let Some(bundle) = factory.bundle.as_deref().filter(|_| !upgraded.is_empty()) {
            // The song's programs of those entries, by whatever path it names them (an alias, `\`).
            let mut again = false;
            for key in SampleCache::referenced(&self.song) {
                let entry = web::needed(bundle, std::slice::from_ref(&key));
                let ours = entry.first().is_some_and(|e| upgraded.contains(&e.id));
                if let (true, SampleKey::File(path)) = (ours, &key)
                    && self.audio.samples.program(path).is_some()
                {
                    self.audio.samples.forget(&key);
                    again = true;
                }
            }
            if again && self.song_download.is_none() {
                self.rebuild_as(tick, Handover::Edit);
            }
        }
        let Some(progress) = self.song_download.as_ref().map(web::Ticket::progress) else {
            return;
        };
        if !progress.done() {
            let title = match progress.retrying {
                true => RETRYING.to_owned(),
                false => downloading(progress.landed, progress.total),
            };
            let shown = |o: &Overlay| matches!(o, Overlay::Busy { title: t } if *t == title);
            if !self.nav.overlays.iter().any(shown) {
                self.navigate(Nav::Open(Overlay::Busy { title }));
            }
            return;
        }
        if let Some(ticket) = self.song_download.take() {
            ticket.keep();
        }
        self.close_busy();
        // Anything built while it waited read the sounds still on their way as missing.
        self.audio.samples = SampleCache::default();
        self.song_audio();
        ctx.request_repaint();
    }
}
