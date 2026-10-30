//! The Save dialog slot: [`crate::ui::save_dialog`] over the file name that `Overlay::SaveDialog` keeps, saving FLM
//! through [`crate::project_files::ProjectFiles::dialog_save`] and exporting the other formats through
//! [`crate::project_files::ProjectFiles::export`]; and the preset dialog's, `Overlay::PresetDialog`, saving through
//! [`crate::project_files::ProjectFiles::save_preset`]; and the audio editor's `New sample name` dialog,
//! `Overlay::SampleName`, naming the edited sample for the app.

use eframe::egui;

use super::{AppCtx, Nav, audio_editor};
use crate::project_files::PresetKind;
use crate::project_files::export::NO_AUDIO_RATE;
use crate::render::{RenderRange, estimated_secs, range_ticks};
use crate::ui::main_menu::library::FORBIDDEN;
use crate::ui::save_dialog::{DialogAction, ExportFormat, RenderPage, SaveDialog};

/// What the song's Save dialog keeps (`Slots::export`).
#[derive(Default)]
pub struct State {
    /// The render options, kept while the app runs (not saved): the last dialog's, which `Quick Render` uses too.
    pub page: RenderPage,
    /// The open dialog, from frame to frame. The app drops it once the dialog is closed, so the next one starts anew
    /// from its `Overlay::SaveDialog` name and [`RenderPage::reopened`].
    pub dialog: Option<SaveDialog>,
    /// The id of the channel whose track menu `Save as MIDI` the open dialog is ([`State::save_as_midi`]); `None`: the
    /// song's dialog.
    pub midi_channel: Option<i32>,
}

impl State {
    /// The track menu's `Save as MIDI` of the channel with id `channel`: the one-format dialog
    /// ([`SaveDialog::save_as_midi`]) proposing `name`, the channel's, made a file name: `/\:*?"<>|` and control
    /// characters become `_` (an imported channel's name has a `:`). Returns the overlay to open.
    pub fn save_as_midi(&mut self, channel: i32, name: &str) -> super::Overlay {
        let named = |c: char| match FORBIDDEN.contains(c) || c.is_control() {
            true => '_',
            false => c,
        };
        let name: String = name.chars().map(named).collect();
        self.dialog = Some(SaveDialog {
            // Kept as they are: this dialog has no render page.
            render: self.page,
            ..SaveDialog::save_as_midi(name.clone())
        });
        self.midi_channel = Some(channel);
        super::Overlay::SaveDialog { name }
    }
}

/// Shows the dialog with `name` in its field. `SAVE` with FLM and a good name and the close X (or Escape) close it
/// (`Nav::Close` closes this overlay, wherever it sits in the stack); a bad name keeps it open under the message. `SAVE`
/// with ZIP or MIDI and `Render` export; the app closes the dialog once the export goes ahead (after the overwrite
/// question, if one is asked). `Render` stores its format for `Quick Render`.
pub fn show(
    state: &mut State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    name: &mut String,
) -> Option<Nav> {
    let page = state.page;
    // A new dialog is the song's.
    if state.dialog.is_none() {
        state.midi_channel = None;
    }
    let dialog = state.dialog.get_or_insert_with(|| SaveDialog {
        render: page.reopened(),
        ..SaveDialog::new(std::mem::take(name))
    });
    let song = &*app.song;
    dialog.song_secs = estimated_secs(song, RenderRange::Full) as f32;
    dialog.selection_secs = range_ticks(song, RenderRange::Selection)
        .map(|_| estimated_secs(song, RenderRange::Selection) as f32);
    dialog.midi_effects = app.settings.midi_note_effects;
    let action = dialog.show(ui.ctx(), &app.cx);
    name.clone_from(&dialog.name);
    state.page = dialog.render;
    app.settings.midi_note_effects = dialog.midi_effects;
    match action? {
        DialogAction::Save(ExportFormat::Flm) => {
            app.files.dialog_save(app.song, name).then_some(Nav::Close)
        }
        DialogAction::Save(format) => {
            // `Apply note effects`: at the engine's rate, as playback prepares the notes.
            let rate = app.audio.sample_rate().unwrap_or(NO_AUDIO_RATE) as f32;
            app.files.midi_note_effects = dialog.midi_effects.then_some(rate);
            match state.midi_channel {
                // FLP's `Include rendered audio` renders its channels first.
                None => app.files.export(app.song, format, name, dialog.flp_audio),
                // `Save as MIDI`: its channel's; a channel gone (an undo under the dialog) closes it.
                Some(id) => {
                    let Some(channel) = app.song.channels.iter().position(|c| c.id == id) else {
                        return Some(Nav::Close);
                    };
                    app.files.save_as_midi(app.song, channel, name);
                }
            }
            None
        }
        DialogAction::Render(_) => {
            app.settings.last_audio_format = dialog.format.id();
            app.files
                .export(app.song, dialog.format, name, dialog.render.split);
            None
        }
        DialogAction::Close => Some(Nav::Close),
    }
}

/// [`show`] for the preset dialog of `kind`: its title and file type, saving the preset [`AppCtx::files`] holds, which
/// the close X and Escape drop. It closes once that is saved, after the overwrite question too.
pub fn show_preset(
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    name: &mut String,
    kind: PresetKind,
) -> Option<Nav> {
    // Saved after the overwrite question, or nothing left to save.
    if !app.files.preset_open() {
        return Some(Nav::Close);
    }
    let mut dialog = SaveDialog {
        preset: Some(kind),
        ..SaveDialog::new(std::mem::take(name))
    };
    let action = dialog.show(ui.ctx(), &app.cx);
    *name = dialog.name;
    match action? {
        DialogAction::Save(_) => app.files.save_preset(name).then_some(Nav::Close),
        DialogAction::Render(_) => None,
        DialogAction::Close => {
            app.files.close_preset();
            Some(Nav::Close)
        }
    }
}

/// [`show`] for the audio editor's `New sample name` dialog: `SAVE` with a good name hands it to the app
/// ([`audio_editor::State::named`]) and closes; a bad one keeps it open behind the message, as the Save dialog does.
/// The close X and Escape close it, which drops the op.
pub fn show_sample(
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    name: &mut String,
    state: &mut audio_editor::State,
) -> Option<Nav> {
    let mut dialog = SaveDialog {
        preset: Some(PresetKind::Sample),
        ..SaveDialog::new(std::mem::take(name))
    };
    let action = dialog.show(ui.ctx(), &app.cx);
    *name = dialog.name;
    match action? {
        DialogAction::Save(_) => state.named = Some(app.files.typed_name(name, "wav")?),
        DialogAction::Render(_) => return None,
        DialogAction::Close => {}
    }
    Some(Nav::Close)
}
