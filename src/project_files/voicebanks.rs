//! User-owned UTAU/OpenUtau voicebank archive imports.

use std::path::{Path, PathBuf};

use super::{IMPORT_FAILED, ProjectFiles, VOICEBANKS_FOLDER};

/// An import is bounded to keep decompression and indexing responsive and safe on mobile devices.
pub(super) const MAX_VOICEBANK_UNPACKED: u64 = 256 << 20;

pub(super) type ImportResult =
    Result<(crate::vocal_synth::VoicebankIndex, Option<Vec<u8>>, PathBuf), String>;

impl ProjectFiles {
    /// System storage entry points shared by the vocal toolbar and preset browser.
    pub fn import_voicebank_bytes(&mut self, path: &Path, bytes: Vec<u8>) {
        self.voicebank_imported(path, bytes);
    }

    pub fn pick_voicebank_folder(&mut self) {
        if !self.provider.opening() && self.copying.is_none() {
            self.voicebank_folder_pick = true;
            self.provider.open_folder();
        }
    }

    pub(super) fn copy_voicebank_folder(&mut self, files: Vec<(String, String)>) {
        let Some(root) = self.voicebanks_dir() else {
            return self.fail(IMPORT_FAILED, "No user storage".into());
        };
        let mut parsed = Vec::new();
        for (name, source) in files {
            let Ok(Some(name)) = crate::zip::clean(&name) else {
                return self.fail(IMPORT_FAILED, "Unsafe bank path".into());
            };
            parsed.push((name, source));
        }
        if parsed.is_empty() || parsed.len() > crate::zip::MAX_ENTRIES {
            return self.fail(IMPORT_FAILED, "Empty or oversized bank folder".into());
        }
        let base = parsed[0]
            .0
            .split('/')
            .next()
            .unwrap_or("Voicebank")
            .to_owned();
        if parsed
            .iter()
            .any(|(p, _)| !p.starts_with(&format!("{base}/")))
        {
            return self.fail(IMPORT_FAILED, "Select one voicebank root folder".into());
        }
        let mut target = root.join(&base);
        let mut n = 1;
        while target.exists() {
            target = root.join(format!("{base} ({n})"));
            n += 1;
        }
        let copies = parsed
            .into_iter()
            .map(|(name, source)| (source, target.join(&name[base.len() + 1..])))
            .collect();
        self.voicebank_folder_copy = Some(target);
        self.copying = Some(Vec::new());
        self.provider.copy(copies);
    }

    /// Index bank metadata on a worker. Never decode/copy WAV audio just to browse a bank.
    pub fn register_voicebank_folder(&mut self, path: &Path) {
        if self.voicebank_import.is_some() {
            return self.fail(IMPORT_FAILED, "A bank import is already running".into());
        }
        let path = path.to_owned();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.voicebank_import = Some(receiver);
        let work = move || {
            let _ = sender.send(index_folder(&path).map(|index| (index, None, path.clone())));
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(error) = std::thread::Builder::new()
            .name("bank-folder-index".into())
            .spawn(work)
        {
            self.voicebank_import = None;
            self.fail(IMPORT_FAILED, error.to_string());
        }
        #[cfg(target_arch = "wasm32")]
        work();
    }

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
        let keep_archive = !self
            .voicebanks_dir()
            .is_some_and(|root| path.starts_with(root));
        let path = path.to_owned();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        self.voicebank_import = Some(receiver);
        let work = move || {
            let fallback = path.file_stem().unwrap_or_default().to_string_lossy();
            let result =
                crate::vocal_synth::index_archive_named(&bytes, MAX_VOICEBANK_UNPACKED, &fallback)
                    .map(|index| (index, keep_archive.then_some(bytes), path.clone()))
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

    pub(super) fn poll_voicebank_import(
        &mut self,
        ctx: &eframe::egui::Context,
        song: &mut crate::song::Song,
    ) {
        let Some(receiver) = &self.voicebank_import else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.voicebank_import = None;
                match result {
                    Ok((index, bytes, source)) => {
                        // Only rename a module still pointing at the completed request.
                        // Changing selection while indexing must not relabel a different bank.
                        let name = index.name.clone();
                        self.store_voicebank_index(index, bytes);
                        for module in song
                            .channels
                            .iter_mut()
                            .flat_map(|c| c.rack.modules.iter_mut())
                        {
                            if module.type_id != 53 {
                                continue;
                            }
                            let selected =
                                module.file.custom_data.as_deref().and_then(|b| {
                                    serde_json::from_slice::<serde_json::Value>(b).ok()
                                });
                            if selected
                                .as_ref()
                                .and_then(|j| j.get("voicebank_source"))
                                .and_then(|s| s.as_str())
                                .is_some_and(|selected| selected == source.to_string_lossy())
                            {
                                module.file.preset_name.clone_from(&name);
                                self.song_changed();
                            }
                        }
                    }
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

    pub(super) fn store_voicebank(
        &mut self,
        index: crate::vocal_synth::VoicebankIndex,
        bytes: Vec<u8>,
    ) {
        self.store_voicebank_index(index, Some(bytes));
    }

    fn store_voicebank_index(
        &mut self,
        index: crate::vocal_synth::VoicebankIndex,
        bytes: Option<Vec<u8>>,
    ) {
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
        if let Some(bytes) = bytes
            && let Err(err) = self.write_user_file(&target, bytes)
        {
            return self.fail(IMPORT_FAILED, format!("{}: {err}", target.display()));
        }
        let registered_name = index.name.clone();
        let aliases = index.entries.len();
        let has_license = index.has_license_document;
        let mut updated = self.voicebanks.clone();
        updated.retain(|bank| bank.name != index.name || bank.oto_path != index.oto_path);
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
            "Registered {registered_name} in {VOICEBANKS_FOLDER}: {aliases} aliases. {}",
            if has_license {
                "A license file is present; verify its terms before sharing or redistributing the bank."
            } else {
                "No license file was found; treat the bank as all-rights-reserved unless its owner says otherwise."
            }
        ));
        self.library_changes += 1;
    }
}

/// Bounded metadata-only directory scan. Symlinks, hidden files and audio payloads are never read.
fn index_folder(root: &Path) -> Result<crate::vocal_synth::VoicebankIndex, String> {
    if std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Bank root cannot be a symlink".into());
    }
    let parent = root.parent().ok_or("Invalid bank root")?;
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    let mut count = 0;
    let mut metadata_budget = 8_u64 << 20;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            count += 1;
            if count > crate::zip::MAX_ENTRIES {
                return Err("Bank folder has too many entries".into());
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let data = if name == "oto.ini"
                || name == "character.txt"
                || name == "install.txt"
                || crate::vocal_synth::license_document(&name)
            {
                let size = entry.metadata().map_err(|e| e.to_string())?.len();
                if size > metadata_budget {
                    return Err("Bank metadata is too large".into());
                }
                metadata_budget -= size;
                super::read_file_limited(
                    std::fs::File::open(entry.path()).map_err(|e| e.to_string())?,
                    size,
                )?
            } else {
                Vec::new()
            };
            files.push(crate::zip::Entry {
                path: entry
                    .path()
                    .strip_prefix(parent)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/"),
                data,
            });
        }
    }
    crate::vocal_synth::index_files(&files)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::project_files::content::tests::Temp;

    #[test]
    fn vocal_import_completion_updates_only_the_still_selected_bank_name() {
        let tmp = Temp::new("vocal-import-display-name");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let source = tmp.0.join("selected.zip");
        let mut song = crate::song::Song::default();
        song.add_channel(crate::song::ChannelKind::Instrument);
        let rack = &mut song.channels[1].rack;
        rack.modules.clear();
        rack.add(53);
        rack.add(53);
        for (i, module) in rack.modules.iter_mut().enumerate() {
            module.file.preset_name = "Pending".into();
            module.file.custom_data = Some(serde_json::to_vec(&serde_json::json!({
                "voicebank_source": if i == 0 { source.clone() } else { tmp.0.join("other.zip") },
                "keep_me": 42
            })).unwrap());
        }
        let index = crate::vocal_synth::index_files(&[
            crate::zip::Entry {
                path: "Bank/oto.ini".into(),
                data: b"a.wav=a,0,0,0,0,0".to_vec(),
            },
            crate::zip::Entry {
                path: "Bank/character.txt".into(),
                data: "\u{feff}name=架空の音声".as_bytes().to_vec(),
            },
        ])
        .unwrap();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        files.voicebank_import = Some(rx);
        tx.send(Ok((index, None, source))).unwrap();
        files.poll_voicebank_import(&eframe::egui::Context::default(), &mut song);
        assert_eq!(
            song.channels[1].rack.modules[0].file.preset_name,
            "架空の音声"
        );
        assert_eq!(song.channels[1].rack.modules[1].file.preset_name, "Pending");
        let data: serde_json::Value = serde_json::from_slice(
            song.channels[1].rack.modules[0]
                .file
                .custom_data
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["keep_me"], 42);
    }

    #[test]
    fn vocal_folder_registration_is_metadata_only_and_survives_restart() {
        let tmp = Temp::new("vocal-folder-register");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        let bank = tmp.0.join(VOICEBANKS_FOLDER).join("UserBank");
        std::fs::create_dir_all(bank.join("C4")).unwrap();
        std::fs::write(bank.join("character.txt"), b"name=UserBank").unwrap();
        std::fs::write(bank.join("C4/oto.ini"), b"a.wav=hello,0,0,0,0,0").unwrap();
        std::fs::write(bank.join("C4/a.wav"), b"not WAV data").unwrap();
        files.register_voicebank_folder(&bank);
        let (index, bytes, _) = files
            .voicebank_import
            .take()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(bytes.is_none());
        assert_eq!(index.entries[0].alias, "hello");
        files.store_voicebank_index(index, bytes);
        let mut reopened = ProjectFiles::on_disk(&tmp.0);
        assert_eq!(reopened.voicebanks().len(), 1);
        assert_eq!(reopened.voicebanks()[0].entries[0].alias, "hello");
        assert!(
            !std::fs::read_dir(tmp.0.join(VOICEBANKS_FOLDER))
                .unwrap()
                .any(|e| e.unwrap().path().extension().is_some_and(|x| x == "zip"))
        );
    }

    #[test]
    fn vocal_folder_import_rejects_traversal_before_copy() {
        let tmp = Temp::new("vocal-folder-traversal");
        let mut files = ProjectFiles::on_disk(&tmp.0);
        files.copy_voicebank_folder(vec![("../outside/oto.ini".into(), "source".into())]);
        assert!(files.voicebank_folder_copy.is_none());
        assert!(files.message.is_some());
    }
}
