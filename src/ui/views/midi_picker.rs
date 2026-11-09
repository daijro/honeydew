//! `Import MIDI Tracks`' picker (a Honeydew addition, #113, in place of the system's file
//! picker): the side browser panel of [`PresetPicker`], titled [`TITLE`], over the whole user folder's MIDI files,
//! opening in `My MIDI`. A pick closes it and goes on as the system picker's file did
//! ([`crate::project_files::ProjectFiles::midi_picked`]): one MIDI channel into the channel at once, more through the
//! MIDI import dialog. `Import`, on every platform, copies a MIDI file from outside the user folder into `My MIDI` as the
//! sample pickers' does, and the open picker lists it. It offers the file browser's ≡ actions on the user's files. The
//! X, a click outside, Escape or the channel being gone close it.

use std::path::Path;

use eframe::egui;

use super::{AppCtx, Nav};
use crate::project_files::{FileKind, MIDI_FOLDER};
use crate::ui::preset_picker::{PickerAction, PresetPicker};

/// The picker's root title (the side panel's titles name what they list).
pub const TITLE: &str = "MIDI Files";

/// The open picker. It starts afresh on each opening, in `My MIDI`.
#[derive(Default)]
pub struct State {
    picker: Option<PresetPicker>,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
    /// [`crate::project_files::ProjectFiles::library_changes`] as it last listed: an `Import` since is listed at once.
    changes: u64,
}

/// Draws the picker for the channel with id `channel`.
pub fn show(state: &mut State, ui: &mut egui::Ui, app: &mut AppCtx, channel: i32) -> Option<Nav> {
    if !app.song.channels.iter().any(|c| c.id == channel) {
        return Some(Nav::Close);
    }
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.picker = None;
    }
    state.pass = pass;
    let exts = FileKind::Midi.extensions();
    let user = app.files.user_dir().map(Path::to_owned);
    let open_at = (state.picker.is_none()).then(|| user.as_ref().map(|u| u.join(MIDI_FOLDER)));
    let picker = state.picker.get_or_insert_with(|| {
        PresetPicker::new(TITLE, user, exts[0], false)
            .of_exts(exts)
            .with_import(true)
    });
    let changes = app.files.library_changes();
    if std::mem::replace(&mut state.changes, changes) != changes {
        picker.reload();
    }
    let (ctx, cx) = (ui.ctx(), &app.cx);
    let shown = app.files.with_preset_files("", exts, |library| {
        if let Some(Some(dir)) = &open_at {
            picker.open_to(library, dir);
        }
        let action = picker.show_mut(ctx, cx, library)?;
        let read = match &action {
            PickerAction::Pick(path) => Some(library.read(path)),
            _ => None,
        };
        Some((action, read))
    });
    // The file actions may have moved or deleted the song's file or folder (`My Files` lists `My Songs`).
    for (from, to) in picker.take_moves() {
        app.files.path_moved(app.song, &from, to.as_deref());
    }
    match shown? {
        (PickerAction::Import, _) => {
            app.files.import_midi();
            None
        }
        (PickerAction::Pick(_), Some(read)) => {
            app.files.midi_picked(channel, read);
            state.picker = None;
            Some(Nav::Close)
        }
        (PickerAction::Close, _) => {
            state.picker = None;
            Some(Nav::Close)
        }
        _ => None,
    }
}
