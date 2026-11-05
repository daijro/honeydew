// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Honeydew (vendored addition): `rfd`'s `FileDialog` for Lemon Studio's plugin editors, whose manifests name this crate
//! `rfd`, so that their own dialog code (`win.dialog(|| rfd::FileDialog::new()….pick_file())`) builds on every platform
//! and asks Honeydew where it answers ([`embed::host_files`]: Android, iOS and the browser, which have no file dialog): a
//! file to open comes from Honeydew's own picker ([`embed::open_file`]), and a file to save is the name the editor
//! proposes, which Honeydew keeps in its user folder when the editor writes it with `os_plugkit::save_file`. Elsewhere
//! it is rfd's dialog. It has what the editors use.

use std::path::PathBuf;

use os_plugkit::embed;

/// A file dialog's settings, shown by [`Self::pick_file`] or [`Self::save_file`].
#[derive(Clone, Debug, Default)]
#[cfg_attr(any(target_arch = "wasm32", target_os = "android", target_os = "ios"), allow(dead_code))]
pub struct FileDialog {
    title: Option<String>,
    filters: Vec<(String, Vec<String>)>,
    file_name: Option<String>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.filters.push((name.into(), extensions.iter().map(ToString::to_string).collect()));
        self
    }

    pub fn set_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    /// The file to open: Honeydew's pick where it answers, else the one picked in rfd's dialog.
    pub fn pick_file(self) -> Option<PathBuf> {
        #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
        if !embed::host_files() {
            return self.rfd().pick_file();
        }
        embed::open_file().map(PathBuf::from)
    }

    /// The file to save: the one named in rfd's dialog; where Honeydew answers, the name proposed (`Untitled` without
    /// one), for `os_plugkit::save_file`.
    pub fn save_file(self) -> Option<PathBuf> {
        #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
        if !embed::host_files() {
            return self.rfd().save_file();
        }
        Some(PathBuf::from(self.file_name.unwrap_or_else(|| "Untitled".into())))
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android", target_os = "ios")))]
    fn rfd(self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(title) = self.title {
            d = d.set_title(title);
        }
        for (name, extensions) in self.filters {
            d = d.add_filter(name, extensions.as_slice());
        }
        if let Some(file_name) = self.file_name {
            d = d.set_file_name(file_name);
        }
        d
    }
}
