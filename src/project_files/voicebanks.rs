//! User-owned UTAU/OpenUtau voicebank archive imports.

use std::path::{Path, PathBuf};

use super::{IMPORT_FAILED, ProjectFiles, VOICEBANKS_FOLDER};

/// An import is bounded to keep decompression and indexing responsive and safe on mobile devices.
const MAX_VOICEBANK_UNPACKED: u64 = 256 << 20;

impl ProjectFiles {
    /// Keeps the original archive in `My Voicebanks` and indexes its oto aliases for the workstation browser.
    pub(super) fn voicebank_imported(&mut self, path: &Path, bytes: Vec<u8>) {
        let _ = self.voicebanks();
        let index = match crate::vocal_synth::index_archive(&bytes, MAX_VOICEBANK_UNPACKED) {
            Ok(index) => index,
            Err(err) => return self.fail(IMPORT_FAILED, format!("{}: {err}", path.display())),
        };
        let folder = self
            .user_dir()
            .map_or_else(|| PathBuf::from(VOICEBANKS_FOLDER), |user| user.join(VOICEBANKS_FOLDER));
        let base = index
            .name
            .chars()
            .map(|ch| if ch.is_control() || matches!(ch, '/' | '\\' | ':') { '-' } else { ch })
            .collect::<String>();
        let base = base.trim_matches([' ', '.', '-']);
        let base = if base.is_empty() { "Voicebank" } else { base };
        let target_name = self.free_name(Some(&folder), base, "zip", None);
        let target = folder.join(format!("{target_name}.zip"));
        if let Err(err) = self.write_user_file(&target, bytes) {
            return self.fail(IMPORT_FAILED, format!("{}: {err}", target.display()));
        }
        let aliases = index.entries.len();
        let has_license = index.has_license_document;
        self.voicebanks.push(index);
        let manifest = folder.join("index.json");
        let saved_index = serde_json::to_vec_pretty(&self.voicebanks);
        if let Err(err) = saved_index
            .map_err(std::io::Error::other)
            .and_then(|bytes| self.write_user_file(&manifest, bytes))
        {
            return self.fail(IMPORT_FAILED, format!("{}: {err}", manifest.display()));
        }
        self.flash = Some(format!(
            "Imported {target_name} to {VOICEBANKS_FOLDER}: {aliases} aliases. {}",
            if has_license {
                "A license file is present; verify its terms before sharing or redistributing the bank."
            } else {
                "No license file was found; treat the bank as all-rights-reserved unless its owner says otherwise."
            }
        ));
        self.library_changes += 1;
    }
}
