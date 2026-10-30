//! The Instruments picker's categories: built from a scan of the instrument presets, not a fixed list.
//!
//! - **Sources** ([`index`]): each instrument's preset folder, its type with it. The Original Content's are `DirectWave
//!   Samples` (DW Sampler) and `Synth Presets2/<Synth>`; ours are `Presets/<module folder>` in the factory
//!   ([`crate::content`]). A source's sub-folders are its categories, renamed by [`category`]; a `Factory` folder is not
//!   indexed. A preset right in the source folder has a category only for Transistor Bass, whose presets have no folder
//!   (`Bass`).
//! - **Items.** The source type's preset files ([`preset_file`]): `.flms`, `.flgsynth`, `.fst`, `.dwp`, and a
//!   `.flmpst` whose `HEAD` names an instrument other than the Slicer ([`listed`]). Each is `<Category>/<Name>`, the
//!   name being the file stem; the first indexed of a `<Category>/<Name>` is the one listed.
//! - **Order.** The categories alphanumeric, without case (the root's "Top Level", [`super::natural`]); a category's
//!   presets in the byte order of their `<Category>/<Name>` path, which no sort changes.
//!
//! **Indexing.**
//! - Our banks are mapped through the same tables. Their folders: MiniSynth `Bass`, `Keys`→`Keyboard`, `Lead`, `Pad`;
//!   GMSynth `Bass`, `Bells`, `Keys`→`Keyboard`, `Lead`, `Pad`; SuperSaw `Bass`, `Lead`, `Pad`, `Pluck`; 3x Osc `Bass`,
//!   `Lead`, `Pad`, `Synth`; Transistor Bass none, so `Bass`. `Pluck` is no Original Content folder and no rename takes
//!   it: it stays ours. Our categories: Bass, Bells, Keyboard, Lead, Pad, Pluck, Synth. Our DW programs are bundled
//!   content, `Presets/DW Sampler/<Category>/<Name>.dwp`, indexed as any DW source; their
//!   Piano joins Keyboard and Strings joins Orchestral; the others add Guitar, Brass, Woodwinds, Choir and Mallets.
//! - The path a category's presets are in byte order of is the virtual `<Category>/<Name>` (the one duplicates are
//!   told by), so the instruments mix in name order.
//! - Index order: the Original Content's sources before ours (ours gives way, as in the module pickers), each group by
//!   the bytes of its folder's path, each folder's files by the bytes of theirs.
//! - A category's own sub-folders are not walked (none of ours has any); a program's sample folder never lists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::params::{Role, module_spec};
use crate::project_files::{
    DWP_EXT, FLGSYNTH_EXT, FLMS_EXT, FST_EXT, PRESET_EXT, SAMPLE_EXTS, SOUNDFONT_EXTS, preset_exts,
};
use crate::ui::main_menu::{SongEntry, SongLibrary};

/// The virtual folder the categories are in: `<` and `>` are forbidden in file names, so no real path is below it.
pub const ROOT: &str = "<instruments>";

/// The files the Instruments picker lists in `My Files`: the instrument files, `.instr` and audio files.
/// A pick of `.instr` or audio loads nothing.
pub const EXTS: [&str; 10] = [
    FLMS_EXT,
    FLGSYNTH_EXT,
    FST_EXT,
    DWP_EXT,
    "instr",
    PRESET_EXT,
    SAMPLE_EXTS[0],
    SAMPLE_EXTS[1],
    SAMPLE_EXTS[2],
    SAMPLE_EXTS[3],
];

const DW_SAMPLER: u16 = 1;
const MINISYNTH: u16 = 0;
const GMSYNTH: u16 = 2;
const TRANSISTOR_BASS: u16 = 19;
const SUPERSAW: u16 = 23;
const SLICER: u16 = 33;
const THREE_OSC: u16 = 39;

/// The instrument types whose presets are indexed, each with its preset folder's name in the Original Content's
/// `Synth Presets2` (`None`: the DW Sampler, whose programs are in `DirectWave Samples`).
pub const SOURCES: [u16; 6] = [
    DW_SAMPLER,
    MINISYNTH,
    GMSYNTH,
    TRANSISTOR_BASS,
    SUPERSAW,
    THREE_OSC,
];

/// A category: its folder row (below [`ROOT`]) and its presets, by their real paths, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    pub folder: SongEntry,
    pub items: Vec<SongEntry>,
}

/// The category the folder `folder` of an instrument of type `type_id` is; `None`: a `Factory` folder, not indexed.
pub fn category(type_id: u16, folder: &str) -> Option<&str> {
    let renamed = match (type_id, folder) {
        (_, "Factory") => return None,
        (DW_SAMPLER, "Piano" | "Synth Key") => "Keyboard",
        (DW_SAMPLER, "Strings") => "Orchestral",
        (DW_SAMPLER, "Synth Lead") => "Lead",
        (DW_SAMPLER, "Synth Pad") => "Pad",
        (DW_SAMPLER, "Synth Bass") => "Bass",
        (MINISYNTH | GMSYNTH, "Keys") => "Keyboard",
        (MINISYNTH | SUPERSAW | GMSYNTH, "Pads") => "Pad",
        (SUPERSAW | GMSYNTH, "Arps" | "Sequences") => "Sequence",
        (SUPERSAW | GMSYNTH, "Effects") => "SFX",
        (SUPERSAW, "Basses") => "Bass",
        (SUPERSAW, "Leads") => "Lead",
        (SUPERSAW, "Synths") => "Synth",
        _ => folder,
    };
    Some(renamed)
}

/// Whether module type `type_id` is an instrument the picker lists presets of: any but the Slicer.
pub fn listed(type_id: u16) -> bool {
    type_id != SLICER && module_spec(type_id).is_some_and(|s| s.role == Role::Instrument)
}

/// The instrument a picked file loads into: by its type (`.flms` MiniSynth, `.flgsynth` GMSynth, `.fst` 3x Osc, `.dwp`
/// DW Sampler), or a `.flmpst`'s by its `HEAD` ([`crate::flm::preset_type`]), when [`listed`]. `None` for anything
/// else (an audio file, `.instr`): its pick does nothing.
pub fn instrument_of(path: &Path, bytes: &[u8]) -> Option<u16> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let type_id = match ext.as_str() {
        FLMS_EXT => MINISYNTH,
        FLGSYNTH_EXT => GMSYNTH,
        FST_EXT => THREE_OSC,
        DWP_EXT => DW_SAMPLER,
        PRESET_EXT => crate::flm::preset_type(bytes)?,
        _ => return None,
    };
    listed(type_id).then_some(type_id)
}

/// The instrument a pick in the Modules browser's Instruments list adds: [`instrument_of`]'s, and the DW Sampler for an
/// `.instr` or audio file (its loader takes the one, refuses the other).
pub fn module_instrument_of(path: &Path, bytes: &[u8]) -> Option<u16> {
    let is = |ext: &str| (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    let dw = is("instr") || SAMPLE_EXTS.iter().any(|x| is(x));
    instrument_of(path, bytes).or(dw.then_some(DW_SAMPLER))
}

/// Whether the file `name` of a source of type `type_id` is one of its presets: one of its preset types; a `.flmpst`
/// only when its `HEAD` names a [`listed`] instrument (`read`).
pub fn preset_file(type_id: u16, name: &Path, read: impl FnOnce() -> Option<Vec<u8>>) -> bool {
    let is = |ext: &str| (name.extension()).is_some_and(|e| e.eq_ignore_ascii_case(ext));
    let own = (preset_exts(type_id).iter())
        .any(|x| is(x) && !SAMPLE_EXTS.contains(x) && !SOUNDFONT_EXTS.contains(x));
    own && (!is(PRESET_EXT) || read().is_some_and(|bytes| lists_preset(&bytes)))
}

/// Whether the module preset `bytes` is a [`listed`] instrument's (its `HEAD`, [`crate::flm::preset_type`]).
pub fn lists_preset(bytes: &[u8]) -> bool {
    crate::flm::preset_type(bytes).is_some_and(listed)
}

/// The categories of `sources` (each a module type and its preset folder), listed through `library`, in index order.
pub fn index(library: &dyn SongLibrary, sources: &[(u16, PathBuf)]) -> Vec<Category> {
    // Category → name → its first preset indexed.
    let mut found: BTreeMap<String, BTreeMap<String, SongEntry>> = BTreeMap::new();
    let list = |dir: &Path| {
        let mut entries = library.list(Some(dir)).unwrap_or_default();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    };
    for (type_id, dir) in sources {
        let mut add = |category: &str, entry: SongEntry| {
            let name = Path::new(&entry.name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned());
            let preset = |name| SongEntry {
                name,
                is_folder: false,
                recency: 0,
                ..entry.clone()
            };
            if let Some(name) = name
                && preset_file(*type_id, Path::new(&entry.name), || {
                    library.read(&entry.path).ok()
                })
            {
                let names = found.entry(category.to_owned()).or_default();
                names.entry(name.clone()).or_insert_with(|| preset(name));
            }
        };
        for entry in list(dir) {
            match (entry.is_folder, category(*type_id, &entry.name)) {
                (true, Some(category)) => {
                    let presets = list(&entry.path).into_iter().filter(|e| !e.is_folder);
                    presets.for_each(|e| add(category, e));
                }
                (false, _) if *type_id == TRANSISTOR_BASS => add("Bass", entry),
                _ => {}
            }
        }
    }
    let mut categories: Vec<Category> = (found.into_iter())
        .map(|(name, items)| Category {
            folder: SongEntry {
                path: Path::new(ROOT).join(&name),
                name,
                is_folder: true,
                recency: 0,
            },
            items: items.into_values().collect(),
        })
        .collect();
    categories.sort_by(|a, b| super::natural(&a.folder.name, &b.folder.name));
    categories
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{PRESETS, ROOT as BUILT_IN, factory};
    use crate::project_files::preset_folder;
    use crate::song::ModuleInstance;

    /// Our own sources, in the factory.
    fn ours() -> Vec<(u16, PathBuf)> {
        let mut sources: Vec<_> = (SOURCES.iter())
            .map(|&t| {
                (
                    t,
                    Path::new(BUILT_IN)
                        .join(PRESETS)
                        .join(preset_folder(t).unwrap()),
                )
            })
            .collect();
        sources.sort_by(|a, b| a.1.cmp(&b.1));
        sources
    }

    fn names(categories: &[Category]) -> Vec<&str> {
        categories.iter().map(|c| c.folder.name.as_str()).collect()
    }

    #[test]
    fn the_renames_are_round_32s_per_instrument() {
        let cases = [
            (DW_SAMPLER, "Synth Key", "Keyboard"),
            (DW_SAMPLER, "Synth Lead", "Lead"),
            (DW_SAMPLER, "Synth Pad", "Pad"),
            (DW_SAMPLER, "Synth Bass", "Bass"),
            (DW_SAMPLER, "Sliced Drums", "Sliced Drums"),
            (MINISYNTH, "Keys", "Keyboard"),
            (MINISYNTH, "Pads", "Pad"),
            (MINISYNTH, "Synth EXP", "Synth EXP"),
            (SUPERSAW, "Arps", "Sequence"),
            (SUPERSAW, "Sequences", "Sequence"),
            (SUPERSAW, "Basses", "Bass"),
            (SUPERSAW, "Leads", "Lead"),
            (SUPERSAW, "Pads", "Pad"),
            (SUPERSAW, "Synths", "Synth"),
            (SUPERSAW, "Effects", "SFX"),
            (SUPERSAW, "Pluck", "Pluck"),
            (GMSYNTH, "Arps", "Sequence"),
            (GMSYNTH, "Sequences", "Sequence"),
            (GMSYNTH, "Keys", "Keyboard"),
            (GMSYNTH, "Pads", "Pad"),
            (GMSYNTH, "Effects", "SFX"),
            (GMSYNTH, "Synth", "Synth"),
            // 3x Osc keeps its folders; a rename is the instrument's own (MiniSynth has no `Leads`).
            (THREE_OSC, "Keys", "Keys"),
            (THREE_OSC, "Drum Kits & FX", "Drum Kits & FX"),
            (MINISYNTH, "Leads", "Leads"),
        ];
        for (t, folder, want) in cases {
            assert_eq!(category(t, folder), Some(want), "{t} {folder}");
        }
        assert_eq!(category(GMSYNTH, "Factory"), None);
    }

    #[test]
    fn our_banks_make_seven_categories_in_alphanumeric_order_with_transistor_bass_under_bass() {
        let categories = index(factory(), &ours());
        assert_eq!(
            names(&categories),
            ["Bass", "Bells", "Keyboard", "Lead", "Pad", "Pluck", "Synth"]
        );
        // Every one of our presets is listed once, under its renamed folder; Transistor Bass's under `Bass`.
        let all: usize = categories.iter().map(|c| c.items.len()).sum();
        assert_eq!(all, 12 + 11 + 14 + 10 + 8);
        let keyboard = &categories[2];
        assert!(
            keyboard
                .items
                .iter()
                .all(|e| e.path.to_string_lossy().contains("/Keys/"))
        );
        assert_eq!(keyboard.items.len(), 6);
        let bass = &categories[0];
        let tb = Path::new(BUILT_IN).join(PRESETS).join("TransistorBass");
        assert_eq!(
            bass.items
                .iter()
                .filter(|e| e.path.starts_with(&tb))
                .count(),
            8
        );
        // In a category, the byte order of `<Category>/<Name>`: names, not the instruments' folders.
        for c in &categories {
            let names: Vec<&str> = c.items.iter().map(|e| e.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(names, sorted, "{}", c.folder.name);
            assert!(
                c.items
                    .iter()
                    .all(|e| !e.is_folder && !e.name.ends_with(".flmpst"))
            );
            assert_eq!(c.folder.path, Path::new(ROOT).join(&c.folder.name));
        }
    }

    /// A fake library of `files` (path → bytes).
    fn fake(files: &[(&str, Vec<u8>)]) -> crate::content::Factory {
        let entries = files.iter().map(|(rel, bytes)| {
            let bytes = bytes.clone();
            crate::content::Entry::new(*rel, move || bytes.clone())
        });
        crate::content::Factory::new(entries.collect())
    }

    fn preset(t: u16) -> Vec<u8> {
        crate::flm::write_preset(&ModuleInstance::new(t, 2)).unwrap()
    }

    #[test]
    fn the_first_indexed_of_a_category_and_name_wins_and_only_instruments_but_the_slicer_list() {
        let library = fake(&[
            ("A/Keys/Twin.flmpst", preset(GMSYNTH)),
            ("A/Keys/Echo.flmpst", preset(35)),
            ("A/Keys/Cut.flmpst", preset(SLICER)),
            ("A/Keys/Notes.txt", b"x".to_vec()),
            ("A/Factory/Hidden.flmpst", preset(GMSYNTH)),
            ("A/Loose.flmpst", preset(GMSYNTH)),
            ("B/Keys/Twin.flms", b"\x99FLS\r\n\x1a\n".to_vec()),
            ("B/Keys/Solo.flms", b"\x99FLS\r\n\x1a\n".to_vec()),
            ("C/Low.flmpst", preset(TRANSISTOR_BASS)),
            ("D/Piano/Grand Piano.dwp", b"DwPr".to_vec()),
            ("D/Keyboard/Electric Piano.dwp", b"DwPr".to_vec()),
            ("D/Strings/Cello.dwp", b"DwPr".to_vec()),
            ("D/Orchestral/Concert Harp.dwp", b"DwPr".to_vec()),
        ]);
        let at = |rel: &str| Path::new(BUILT_IN).join(rel);
        let sources = [
            (GMSYNTH, at("A")),
            (MINISYNTH, at("B")),
            (TRANSISTOR_BASS, at("C")),
            (DW_SAMPLER, at("D")),
        ];
        let categories = index(&library, &sources);
        assert_eq!(names(&categories), ["Bass", "Keyboard", "Orchestral"]);
        let keyboard: Vec<(&str, PathBuf)> = (categories[1].items.iter())
            .map(|e| (e.name.as_str(), e.path.clone()))
            .collect();
        assert_eq!(
            keyboard,
            [
                ("Electric Piano", at("D/Keyboard/Electric Piano.dwp")),
                ("Grand Piano", at("D/Piano/Grand Piano.dwp")),
                ("Solo", at("B/Keys/Solo.flms")),
                ("Twin", at("A/Keys/Twin.flmpst")),
            ]
        );
        assert_eq!(categories[0].items[0].path, at("C/Low.flmpst"));
        assert_eq!(
            categories[2]
                .items
                .iter()
                .map(|e| e.path.clone())
                .collect::<Vec<_>>(),
            [
                at("D/Strings/Cello.dwp"),
                at("D/Orchestral/Concert Harp.dwp")
            ]
        );
    }

    #[test]
    fn the_categories_sort_without_case() {
        // GMSynth's `Effects` and `Arps` are `SFX` and `Sequence`: `Se` before `SF`, as `e` before `f` without case.
        let library = fake(&[
            ("G/Effects/Zap.flmpst", preset(GMSYNTH)),
            ("G/Arps/Run.flmpst", preset(GMSYNTH)),
            ("G/bells/Ding.flmpst", preset(GMSYNTH)),
        ]);
        let sources = [(GMSYNTH, Path::new(BUILT_IN).join("G"))];
        assert_eq!(
            names(&index(&library, &sources)),
            ["bells", "Sequence", "SFX"]
        );
    }

    #[test]
    fn a_pick_loads_into_its_files_instrument_and_audio_or_instr_into_none() {
        let file = Path::new;
        assert_eq!(instrument_of(file("a.flms"), b""), Some(MINISYNTH));
        assert_eq!(instrument_of(file("a.FLGSYNTH"), b""), Some(GMSYNTH));
        assert_eq!(instrument_of(file("a.fst"), b""), Some(THREE_OSC));
        assert_eq!(instrument_of(file("a.dwp"), b"DwPr"), Some(DW_SAMPLER));
        assert_eq!(
            instrument_of(file("a.flmpst"), &preset(SUPERSAW)),
            Some(SUPERSAW)
        );
        assert_eq!(instrument_of(file("a.flmpst"), &preset(52)), Some(52));
        assert_eq!(instrument_of(file("a.flmpst"), &preset(SLICER)), None);
        assert_eq!(instrument_of(file("a.flmpst"), &preset(35)), None);
        assert_eq!(instrument_of(file("a.wav"), b"RIFF"), None);
        assert_eq!(instrument_of(file("a.instr"), b""), None);
        assert!(listed(TRANSISTOR_BASS) && listed(52) && !listed(SLICER) && !listed(35));
    }
}
