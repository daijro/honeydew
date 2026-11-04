//! The theme creator's files (#67). The user's themes are `<name>.json` files, the theme JSON of `colors.md`, in the
//! user folder's [`THEMES_FOLDER`]: `Save` writes there, and the `Theme:` list and `Load themes` read them back.
//! `Export` writes a copy through the provider, as a song's `Save as` does: the desktop's save dialog, a download on
//! the web, the themes folder on Android. `Import` picks a file with the provider's picker, which the creator then
//! reads ([`crate::theme::import`]) and saves.

use std::path::{Path, PathBuf};

use super::{FileEvent, Pick, ProjectFiles, SAVE_FAILED, stem};
use crate::theme::{THEMES_FOLDER, Theme, import};
use crate::ui::main_menu::library::check_folder_name;

impl ProjectFiles {
    /// The user's themes, in file name order. A file that is no theme is left out.
    pub fn user_themes(&self) -> Vec<Theme> {
        let Some(dir) = self.themes_dir() else {
            return Vec::new();
        };
        (self.user_files(&dir, "json").iter())
            .filter_map(|path| import(&stem(path), &self.read_user_file(path).ok()?).ok())
            .collect()
    }

    /// Writes `theme` as `<its name>.json` into the themes folder, over a file of that name. Where it went, or why
    /// not.
    pub fn save_theme(&mut self, theme: &Theme) -> Result<PathBuf, String> {
        let path = self.theme_path(theme)?;
        let bytes = theme.to_json().into_bytes();
        self.write_user_file(&path, bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// `Export`: `theme` as `<its name>.json` where the user puts it (the provider's save, [`Self::theme_exported`]
    /// flashes where). Fails at once on a name that is no file name.
    pub fn export_theme(&mut self, theme: &Theme) -> Result<(), String> {
        let path = self.theme_path(theme)?;
        self.theme_export = true;
        self.provider
            .save(&path, true, theme.to_json().into_bytes());
        Ok(())
    }

    /// `Import`: the file picker, for a theme file ([`Self::take_theme_pick`]).
    pub fn pick_theme(&mut self) {
        self.open_pick(Pick::Theme);
    }

    /// The theme file `Import` picked, once: its path (on the web its name) and bytes.
    pub fn take_theme_pick(&mut self) -> Option<(PathBuf, Vec<u8>)> {
        self.theme_pick.take()
    }

    fn themes_dir(&self) -> Option<PathBuf> {
        self.user.as_ref().map(|user| user.join(THEMES_FOLDER))
    }

    fn theme_path(&self, theme: &Theme) -> Result<PathBuf, String> {
        let file = format!("{}.json", theme.name.trim());
        check_folder_name(&file).map_err(|e| e.to_string())?;
        let dir = self.themes_dir().ok_or("There is no user folder.")?;
        Ok(dir.join(file))
    }

    /// `event` finishes the export under way: the next `.json` save to end (written wherever the desktop's dialog put
    /// it, failed, or its dialog cancelled). No other save writes JSON.
    pub(super) fn is_theme_export(&self, event: &FileEvent) -> bool {
        let json = |p: &Path| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        };
        self.theme_export
            && match event {
                FileEvent::Saved(path) | FileEvent::SaveCancelled(path) => json(path),
                FileEvent::Failed(SAVE_FAILED, detail) => {
                    (detail.split_once(": ")).is_some_and(|(path, _)| json(Path::new(path)))
                }
                _ => false,
            }
    }

    pub(super) fn theme_exported(&mut self, event: FileEvent) {
        self.theme_export = false;
        match event {
            FileEvent::Saved(path) => {
                self.flash = Some(format!("Saved theme to {}", path.display()));
            }
            FileEvent::Failed(_, err) => self.fail("Could not save theme", err),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_files::mem::MemFiles;
    use crate::theme::{USER_THEME_ID, theme_by_id};

    #[test]
    fn a_saved_theme_lists_back_as_it_was() {
        let mem = MemFiles::default();
        mem.0.borrow_mut().save_dir = Some("Exports".into());
        let mut files = ProjectFiles::with_provider(Box::new(mem.clone()));
        let theme = Theme {
            id: USER_THEME_ID,
            name: "Sunset".into(),
            ..theme_by_id(3)
        };
        let path = files.save_theme(&theme).unwrap();
        assert_eq!(path, Path::new(THEMES_FOLDER).join("Sunset.json"));
        assert_eq!(files.user_themes(), std::slice::from_ref(&theme));
        // A JSON file that is no theme is not listed; a name that is no file name is refused.
        let other = Path::new(THEMES_FOLDER).join("notes.json");
        files.write_user_file(&other, b"{}".to_vec()).unwrap();
        assert_eq!(files.user_themes().len(), 1);
        let slash = Theme {
            name: "a/b".into(),
            ..theme.clone()
        };
        assert!(files.save_theme(&slash).is_err());
        // `Export` goes where the dialog says; that save is the theme's, not the song's.
        let mut song = crate::song::Song {
            title: "Song".into(),
            ..Default::default()
        };
        files.export_theme(&theme).unwrap();
        assert_eq!(files.poll(&mut song), None);
        let flash = files.take_flash();
        assert_eq!(flash.as_deref(), Some("Saved theme to Exports/Sunset.json"));
        assert_eq!(song.title, "Song");
        assert!(files.export_theme(&slash).is_err());
        // A cancelled dialog ends the export too: nothing waits for it.
        mem.0.borrow_mut().save_dir = None;
        files.export_theme(&theme).unwrap();
        assert_eq!(files.poll(&mut song), None);
        assert!(!files.theme_export);
        assert_eq!((files.take_flash(), files.take_overlay()), (None, None));
    }
}
