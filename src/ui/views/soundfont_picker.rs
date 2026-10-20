//! The SoundFont picker overlay: the SoundFont Player's `Patch` spinner (or, in the error state, a tap on its panel)
//! opens it over `My SoundFonts` (and imported content's `SoundFonts`, when there), starting in the patches of the
//! module's SoundFont when the rack read them, its current patch current. A patch pick sets the module's `SF2D` (the
//! SoundFont's shorthand path and the patch index) at once, as one final song edit: a new engine, and no undo step (the
//! rack is not covered by undo); the spinner shows it and the picker stays open. The patches' `Sort` is stored as
//! `SoundFontPatchOrdering` at once. A SoundFont that cannot be read loads too, at patch 0, the picker closes and the
//! panel shows its error. Playing it is the engine's.
//!
//! A patch pick does not turn on an audition strip: the patches are no presets, and the keyboard plays the channel.

use eframe::egui;

use super::{AppCtx, Nav, rack};
use crate::flm::sf2d::Sf2d;
use crate::panels::inst::soundfont_player;
use crate::project_files::{SF2_EXT, SOUNDFONTS};
use crate::ui::soundfont_picker::{SoundFontAction, SoundFontPicker};

/// The open picker. It starts afresh on each opening, however it was last closed.
#[derive(Default)]
pub struct State {
    picker: Option<SoundFontPicker>,
    /// The egui pass it was last drawn in: a gap means it was closed in between.
    pass: u64,
}

/// Draws the picker for the SoundFont Player `module` on `channel`, modal, over the left of the screen, and loads its
/// pick, telling `rack` the patches it read so the panel needs no second read. The X, a click outside, Escape, a
/// SoundFont that fails, or the module being gone close it.
pub fn show(
    state: &mut State,
    rack: &mut rack::State,
    ui: &mut egui::Ui,
    app: &mut AppCtx,
    channel: usize,
    module: i32,
) -> Option<Nav> {
    let player = app.song.find_module(channel, module);
    let Some(player) = player.filter(|m| m.type_id == soundfont_player::TYPE_ID) else {
        return Some(Nav::Close);
    };
    let pass = ui.ctx().cumulative_pass_nr();
    if state.pass + 1 != pass {
        state.picker = None;
    }
    state.pass = pass;
    let files = &*app.files;
    let picker = state.picker.get_or_insert_with(|| {
        // The module's SoundFont, as the rack read it (it drew the card that opened this), at the path the library
        // lists it under.
        let sf2d = player.file.custom_data.as_deref().and_then(Sf2d::read);
        let current = sf2d.and_then(|s| {
            let patches = rack.soundfont(&s.path)?.as_ref().ok()?.clone();
            // An index past either end is the patch at that end, as the spinner reads it.
            let patch = usize::try_from(s.patch)
                .unwrap_or(0)
                .min(patches.len().saturating_sub(1));
            Some((files.library_path(&s.path)?, patches, patch))
        });
        let mut picker = SoundFontPicker::new(files.soundfonts_dir(), current)
            .with_factory(files.content_folder(SOUNDFONTS));
        picker.sort = app.settings.patch_ordering();
        picker
    });
    let library = files.preset_library(&[SF2_EXT]);
    let action = picker.show(ui.ctx(), &app.cx, &*library);
    drop(library);
    // A pick from `Sort`'s menu is saved as `SoundFontPatchOrdering`, the active one too.
    if let Some(sort) = picker.sort_picked.take() {
        app.settings.sound_font_patch_ordering = sort.id();
    }
    let (path, patch, read) = match action? {
        SoundFontAction::Close => {
            state.picker = None;
            return Some(Nav::Close);
        }
        SoundFontAction::Patch {
            path,
            patches,
            index,
        } => (path, index, Ok(patches)),
        SoundFontAction::Failed { path, error } => {
            state.picker = None;
            (path, 0, Err(error))
        }
    };
    let stays = read.is_ok();
    // The picker lists the user and content folders only, so a pick always has a shorthand; one without loads nothing.
    let Some(shorthand) = app.files.shorthand(&path) else {
        state.picker = None;
        return Some(Nav::Close);
    };
    rack.soundfont_read(shorthand.clone(), read);
    let sf2d = Sf2d {
        path: shorthand,
        patch: patch as i32,
    };
    // A pick tries the SoundFonts and samples missing before again (as a sample pick does): re-picking the same file
    // and patch rebuilds when the engine had no SoundFont there.
    let missing = app.audio.samples.soundfont(&sf2d.path).is_none();
    app.audio.samples.forget_missing();
    let player = (app.song.channels.get_mut(channel)).and_then(|c| c.rack.get_mut(module));
    if let Some(player) = player
        && let Ok(blob) = sf2d.write(player.file.custom_data.as_deref())
    {
        let changed = player.file.custom_data.as_ref() != Some(&blob);
        player.file.custom_data = Some(blob);
        if changed || missing {
            app.song_edited(true);
        }
    }
    (!stays).then_some(Nav::Close)
}
