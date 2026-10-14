//! The MIDI import dialog's slot: [`crate::ui::midi_import`] over the file the track menu's `Import MIDI Tracks`
//! picked. `Import` hands the file and the choices to the app ([`State::take_import`]), which imports them into the
//! channel whose menu it was as one undo step (`app/tracks.rs`).

use eframe::egui;

use super::{AppCtx, Nav};
use crate::project_files::MidiTracks;
use crate::smf::import::ImportOpts;
use crate::ui::midi_import::{MidiImport, MidiImportAction};

/// The open dialog and its file; then, once `Import` is tapped, the import for the app to run.
#[derive(Default)]
pub struct State {
    open: Option<(MidiTracks, MidiImport)>,
    chosen: Option<(MidiTracks, ImportOpts)>,
}

impl State {
    /// The dialog for `picked`, for `Overlay::MidiImport` to show.
    pub fn open(&mut self, picked: MidiTracks) {
        let dialog = MidiImport::new(picked.scan.clone());
        self.open = Some((picked, dialog));
    }

    /// The file and the choices `Import` took, once.
    pub fn take_import(&mut self) -> Option<(MidiTracks, ImportOpts)> {
        self.chosen.take()
    }
}

/// Shows the dialog; `Import` and `Cancel` close it. With no file it closes at once.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx) -> Option<Nav> {
    let Some((_, dialog)) = &mut state.open else {
        return Some(Nav::Close);
    };
    let action = dialog.show(ui.ctx(), &app.cx, app.regions.area)?;
    let (picked, _) = state.open.take()?;
    if let MidiImportAction::Import(opts) = action {
        state.chosen = Some((picked, opts));
    }
    Some(Nav::Close)
}
