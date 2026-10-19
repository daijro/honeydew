//! The exports in the app ([`crate::project_files::export`]): an export that goes ahead closes the Save dialog,
//! and an audio one renders behind the busy screen (`Overlay::Busy`), one slice a frame before the screens, with the
//! transport stopped.

use eframe::egui;

use super::HoneydewApp;
use crate::project_files::export::{Done, Job, NO_AUDIO_RATE, Split, slice};
use crate::sample_cache;
use crate::ui::views::{Nav, Overlay};

fn dialog(o: &Overlay) -> bool {
    matches!(o, Overlay::SaveDialog { .. })
}

impl HoneydewApp {
    /// After the files, each frame: a closed Save dialog lets go of its state. An export that went ahead closes the
    /// dialog; an audio one starts rendering with the page's options, the samples loaded as a rebuild loads them, at
    /// The engine rate (no resampling; 44 100 Hz without an output).
    pub(super) fn export_started(&mut self, ctx: &egui::Context) {
        let Some(export) = self.files.take_export() else {
            if !self.nav.overlays.iter().any(dialog) {
                self.slots.export.dialog = None;
            }
            return;
        };
        self.nav.overlays.retain(|o| !dialog(o));
        self.slots.export.dialog = None;
        ctx.request_repaint();
        if !export.format.is_audio() && export.split != Split::Flp {
            return;
        }
        self.audio
            .samples
            .load(&self.song, &sample_cache::reader(&self.files));
        let rate = self.audio.sample_rate().unwrap_or(NO_AUDIO_RATE);
        let page = self.slots.export.page;
        match Job::new(&self.song, &self.audio.samples, export, page, rate) {
            Ok(job) => {
                self.navigate(Nav::Open(Overlay::Busy { title: job.title() }));
                self.export = Some(job);
            }
            Err(error) => self.files.render_failed(&error),
        }
    }

    /// Before the screens: a slice of the render under way, its stage's title on the busy screen (a stem's under
    /// `Exporting channel %d: %s`), and the transport stopped (its Stop goes out after the screens, as the play
    /// button's does). Each stem is written once encoded, and the file once the job is over; an error shows its message.
    pub(super) fn export_slice(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.export else {
            return;
        };
        self.main_screen.playing = false;
        ctx.request_repaint();
        // A stem that could not be written ends the export; its message is up.
        if !self.files.stem_write_failed() {
            let done = job.step(&mut slice());
            let (export, title) = (job.export().clone(), job.title());
            match done {
                None => return self.navigate(Nav::Open(Overlay::Busy { title })),
                Some(Done::Stem(name, bytes)) => {
                    self.files.write_stem(&export, &name, bytes);
                    return self.navigate(Nav::Open(Overlay::Busy { title }));
                }
                Some(Done::File(bytes)) => self.files.write_export(&export, bytes),
                Some(Done::Stems) => self.files.stems_rendered(),
                Some(Done::Rendered(song, stems)) => {
                    self.files.flp_rendered(&song, &export, &stems)
                }
                Some(Done::Failed(error)) => self.files.render_failed(&error),
            }
        }
        self.export = None;
        self.nav
            .overlays
            .retain(|o| !matches!(o, Overlay::Busy { .. }));
    }
}
