//! Sylenth1 presets in Rind, through the preset picker the module already has. The parsing and the parameter mapping are
//! `rind::sylenth`; this is how a pick, an import and a save meet the app's files.
//!
//! - **A pick.** Rind's picker lists `.fxp` programs and `.fxb` banks beside its own `.flmpst` presets
//!   ([`super::preset_exts`]). A program loads into the module ([`load`]): the mapped values become the module's params,
//!   its name and category (`Sylenth1`) the preset's. A bank loads its first program. A damaged or foreign file is an
//!   error that changes nothing. The parse runs on the UI thread, never the audio thread: a bank is read whole in a few
//!   milliseconds, and the sound only ever sees the module's params.
//! - **Importing a bank.** Picking a bank also writes each of its programs as a native preset into
//!   `My Presets/Rind/Sylenth1/<bank>/` ([`ProjectFiles::sylenth_picked`]), so the picker browses and searches them as
//!   any presets, and a pick of one never reads the original again. A file that is already there stays as it is: nothing
//!   is overwritten, and a second import of the same bank writes nothing new.
//! - **What differs** is said, not hidden: the flash after a pick names what Rind approximates or has no equivalent for
//!   ([`rind::presets::Preset::warnings`]).
//! - **Saving** is the card's own `Save preset` (`.flmpst`), which a Rind card has like any module with presets. There is
//!   no export to Sylenth1's formats, and the original files are only ever read.

use std::collections::HashSet;
use std::io;
use std::path::Path;

use rind::presets::Preset;

use super::ProjectFiles;
use super::presets::{FXB_EXT, FXP_EXT, PRESET_EXT};
use crate::song::ModuleInstance;
use crate::song::plugin::PLUGIN_INSTRUMENT;

/// The category an imported preset has, and its folder in Rind's presets.
pub const CATEGORY: &str = "Sylenth1";
/// The flash lines a pick shows, at most.
const FLASH_LINES: usize = 3;
/// The longest file name an imported program gets.
const NAME_LIMIT: usize = 80;

/// `path` is a Sylenth1 program or bank, by its extension.
pub fn is_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(FXP_EXT) || e.eq_ignore_ascii_case(FXB_EXT))
}

/// `module` is Rind's.
pub fn is_rind(module: &ModuleInstance) -> bool {
    module.type_id == PLUGIN_INSTRUMENT
        && crate::ui::rack::lemon_plugin(module).is_some_and(|b| b.clap_id() == rind::CLAP_ID)
}

/// Sets `module`'s params to `preset`'s values (one per Rind parameter after its Enable) and names it. Its Enable, key
/// range and place in the rack stay.
pub fn apply(module: &mut ModuleInstance, preset: &Preset) {
    let norm = preset.normalized();
    if module.params.len() < 1 + norm.len() {
        module.params.resize(1 + norm.len(), 0.0);
    }
    for (slot, value) in module.params[1..].iter_mut().zip(norm) {
        *slot = value;
    }
    let file = &mut module.file;
    file.preset_name = preset.name.clone();
    file.category = CATEGORY.to_owned();
    file.provenance = format!("{CATEGORY}/{}", preset.name);
}

/// Loads the first program of the Sylenth1 file `bytes` into `module`; the program's warnings. An error leaves
/// `module` as it was.
pub fn load(module: &mut ModuleInstance, bytes: &[u8]) -> Result<Vec<String>, String> {
    if !is_rind(module) {
        return Err("Sylenth1 presets load into Rind only.".to_owned());
    }
    let programs = rind::sylenth::parse(bytes).map_err(|e| e.to_string())?;
    let first = programs
        .first()
        .ok_or_else(|| "The file holds no presets.".to_owned())?;
    apply(module, first);
    Ok(first.warnings.clone())
}

/// A file name for a program name: the characters a file name cannot hold replaced, trimmed, never empty.
pub fn safe_name(name: &str, fallback: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed: String = mapped
        .trim()
        .trim_matches('.')
        .trim()
        .chars()
        .take(NAME_LIMIT)
        .collect();
    match trimmed.trim() {
        "" => fallback.to_owned(),
        name => name.to_owned(),
    }
}

/// The lines a pick's flash shows for `warnings`: what is specific (the estimate note aside), the first few, then how
/// many more.
pub fn flash_lines(prefix: &str, warnings: &[String]) -> String {
    let specific: Vec<&String> = warnings
        .iter()
        .filter(|w| w.as_str() != rind::sylenth::APPROX)
        .collect();
    let mut lines = vec![prefix.to_owned()];
    lines.extend(specific.iter().take(FLASH_LINES).map(|w| (*w).clone()));
    if specific.len() > FLASH_LINES {
        lines.push(format!("and {} more", specific.len() - FLASH_LINES));
    }
    lines.join("\n")
}

/// What importing a bank did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub programs: usize,
    pub written: usize,
    pub skipped: usize,
}

impl ProjectFiles {
    /// Writes each of `programs` as a native Rind preset into `My Presets/Rind/Sylenth1/<bank>/`, from `template` (a Rind
    /// module: its plugin reference makes the file a Rind preset). A file already there is left alone. Never into the
    /// original content's folder.
    pub fn import_sylenth_bank(
        &mut self,
        bank: &str,
        template: &ModuleInstance,
        programs: &[Preset],
    ) -> io::Result<Imported> {
        let base = self
            .presets_dir(PLUGIN_INSTRUMENT)
            .ok_or(io::ErrorKind::NotFound)?;
        let dir = base.join(CATEGORY).join(safe_name(bank, "Bank"));
        let mut used = HashSet::new();
        let mut done = Imported {
            programs: programs.len(),
            ..Imported::default()
        };
        for (i, program) in programs.iter().enumerate() {
            let stem = safe_name(&program.name, &format!("Program {}", i + 1));
            let name = std::iter::once(stem.clone())
                .chain((2..).map(|n| format!("{stem} ({n})")))
                .find(|name| used.insert(name.to_lowercase()))
                .expect("an unbounded range has a free number");
            let path = dir.join(format!("{name}.{PRESET_EXT}"));
            if self.taken(&path) {
                done.skipped += 1;
                continue;
            }
            let mut module = template.clone();
            module.instance_id = 0;
            module.minimized = false;
            module.sidechain = None;
            apply(&mut module, program);
            module.name_preset(&path, &base);
            // The program's own name, not the file's, when they differ by the characters a file name cannot hold.
            module.file.preset_name = program.name.clone();
            let bytes =
                crate::flm::write_preset(&module).map_err(|e| io::Error::other(e.to_string()))?;
            self.write_user_file(&path, bytes)?;
            done.written += 1;
        }
        Ok(done)
    }

    /// A Sylenth1 file `path` (its bytes `bytes`) was picked for a Rind module like `template`, and its first program
    /// loaded: a bank's programs become native presets ([`Self::import_sylenth_bank`]), and a flash says what differs.
    pub fn sylenth_picked(&mut self, template: &ModuleInstance, path: &Path, bytes: &[u8]) {
        let Ok(programs) = rind::sylenth::parse(bytes) else {
            return;
        };
        let Some(first) = programs.first() else {
            return;
        };
        let bank = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_bank = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(FXB_EXT));
        if !is_bank {
            let prefix = if first.warnings.iter().any(|w| w != rind::sylenth::APPROX) {
                "Imported from Sylenth1; some settings differ:"
            } else {
                "Imported from Sylenth1"
            };
            self.flash(flash_lines(prefix, &first.warnings));
            return;
        }
        match self.import_sylenth_bank(&bank, template, &programs) {
            Ok(done) => {
                let differing = programs
                    .iter()
                    .filter(|p| p.warnings.iter().any(|w| w != rind::sylenth::APPROX))
                    .count();
                let mut lines = vec![format!(
                    "Imported {} of {} presets into My Presets/Rind/{CATEGORY}/{}",
                    done.written + done.skipped,
                    done.programs,
                    safe_name(&bank, "Bank")
                )];
                if differing > 0 {
                    lines.push(format!(
                        "{differing} have settings Rind approximates or does not have"
                    ));
                }
                self.flash(lines.join("\n"));
            }
            Err(err) => self.flash(format!("Could not import the bank: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sylenth_files_are_recognised_by_their_extension() {
        for name in ["a.fxp", "A.FXP", "bank.fxb", "x/y.Fxb"] {
            assert!(is_file(Path::new(name)), "{name}");
        }
        for name in ["a.flmpst", "fxp", "a.fxp.txt", "a"] {
            assert!(!is_file(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn file_names_lose_only_what_a_file_name_cannot_hold() {
        assert_eq!(safe_name("Deep Bass", "x"), "Deep Bass");
        assert_eq!(safe_name("A/B:C*D", "x"), "A_B_C_D");
        assert_eq!(safe_name("  ..  ", "Program 3"), "Program 3");
        assert_eq!(safe_name("", "Program 1"), "Program 1");
        assert_eq!(safe_name(&"x".repeat(200), "p").chars().count(), NAME_LIMIT);
    }

    #[test]
    fn the_flash_names_what_differs_and_counts_the_rest() {
        let w: Vec<String> = std::iter::once(rind::sylenth::APPROX.to_owned())
            .chain((1..=5).map(|i| format!("w{i}")))
            .collect();
        let text = flash_lines("Head", &w);
        assert_eq!(text, "Head\nw1\nw2\nw3\nand 2 more");
        assert_eq!(
            flash_lines("Head", &[rind::sylenth::APPROX.to_owned()]),
            "Head"
        );
    }
}
