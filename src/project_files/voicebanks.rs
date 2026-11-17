//! User-owned UTAU/OpenUtau voicebank archive imports.

use std::path::{Path, PathBuf};

use super::{IMPORT_FAILED, ProjectFiles, VOICEBANKS_FOLDER};

/// An import is bounded to keep decompression and indexing responsive and safe on mobile devices.
const MAX_VOICEBANK_UNPACKED: u64 = 256 << 20;

pub(super) type ImportResult = Result<(crate::vocal_synth::VoicebankIndex, Vec<u8>), String>;

impl ProjectFiles {
    /// A bank is being validated on the import worker.
    pub fn importing_voicebank(&self) -> bool {
        self.voicebank_import.is_some()
    }

    /// Validate/decompress away from the native UI/audio threads. A single in-flight bank bounds memory.
    pub(super) fn voicebank_imported(&mut self, path: &Path, bytes: Vec<u8>) {
        if self.voicebank_import.is_some() {
            return self.fail(
                IMPORT_FAILED,
                "A voicebank import is already running".into(),
            );
        }
        let path = path.to_owned();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.voicebank_import = Some(receiver);
        let work = move || {
            let result = crate::vocal_synth::index_archive(&bytes, MAX_VOICEBANK_UNPACKED)
                .map(|index| (index, bytes))
                .map_err(|err| format!("{}: {err}", path.display()));
            let _ = sender.send(result);
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(err) = std::thread::Builder::new()
            .name("voicebank-import".into())
            .spawn(work)
        {
            self.voicebank_import = None;
            self.fail(IMPORT_FAILED, err.to_string());
        }
        #[cfg(target_arch = "wasm32")]
        work();
    }

    pub(super) fn poll_voicebank_import(&mut self, ctx: &eframe::egui::Context) {
        let Some(receiver) = &self.voicebank_import else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.voicebank_import = None;
                match result {
                    Ok((index, bytes)) => self.store_voicebank(index, bytes),
                    Err(err) => self.fail(IMPORT_FAILED, err),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.voicebank_import = None;
                self.fail(
                    IMPORT_FAILED,
                    "The voicebank import worker stopped unexpectedly".into(),
                );
            }
        }
    }

    /// The browser's permanent root, even before the first voicebank import.
    pub fn voicebanks_dir(&self) -> Option<PathBuf> {
        self.user_dir().map(|user| user.join(VOICEBANKS_FOLDER))
    }

    fn store_voicebank(&mut self, index: crate::vocal_synth::VoicebankIndex, bytes: Vec<u8>) {
        let _ = self.voicebanks();
        let folder = self.user_dir().map_or_else(
            || PathBuf::from(VOICEBANKS_FOLDER),
            |user| user.join(VOICEBANKS_FOLDER),
        );
        let base = index
            .name
            .chars()
            .map(|ch| {
                if ch.is_control() || matches!(ch, '/' | '\\' | ':') {
                    '-'
                } else {
                    ch
                }
            })
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
        let mut updated = self.voicebanks.clone();
        updated.push(index);
        let manifest = folder.join("index.json");
        let saved_index = serde_json::to_vec_pretty(&updated);
        if let Err(err) = saved_index
            .map_err(std::io::Error::other)
            .and_then(|bytes| self.write_user_file(&manifest, bytes))
        {
            return self.fail(IMPORT_FAILED, format!("{}: {err}", manifest.display()));
        }
        self.voicebanks = updated;
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
