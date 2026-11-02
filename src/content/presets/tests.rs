use std::collections::HashSet;

use super::*;
use crate::content::factory;
use crate::flm::{preset_fits, read_preset};
use crate::panels::cstm::settings_from_cstm;
use crate::ui::main_menu::SongLibrary;

/// The bank of module type `type_id`.
fn bank(type_id: u16) -> &'static [Preset] {
    banks().find(|&(t, _)| t == type_id).expect("a bank").1
}

/// How many presets of `bank` are in each category, in index order.
fn counts(bank: &[Preset]) -> Vec<(&'static str, usize)> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for preset in bank {
        match counts.iter_mut().find(|(c, _)| *c == preset.category) {
            Some((_, n)) => *n += 1,
            None => counts.push((preset.category, 1)),
        }
    }
    counts.sort();
    counts
}

#[test]
fn the_banks_hold_the_planned_presets_per_category() {
    assert_eq!(
        counts(bank(0)),
        [("Bass", 3), ("Keys", 3), ("Lead", 3), ("Pad", 3)]
    );
    assert_eq!(
        counts(bank(23)),
        [("Bass", 2), ("Lead", 3), ("Pad", 3), ("Pluck", 3)]
    );
    assert_eq!(counts(bank(19)), [("", 8)]);
    assert_eq!(
        counts(bank(2)),
        [
            ("Bass", 3),
            ("Bells", 2),
            ("Keys", 3),
            ("Lead", 3),
            ("Pad", 3)
        ]
    );
    assert_eq!(
        counts(bank(39)),
        [("Bass", 3), ("Lead", 3), ("Pad", 2), ("Synth", 2)]
    );
}

#[test]
fn every_param_name_resolves_to_exactly_one_index_and_every_value_is_normalized() {
    for (t, bank) in banks() {
        let spec = module_spec(t).expect("a module");
        for preset in bank {
            let mut seen = HashSet::new();
            for &(name, value) in preset.params {
                let at = format!("{} {}: {name}", spec.display_name, preset.name);
                let index = resolve(spec, name)
                    .unwrap_or_else(|| panic!("{at} resolves to none or several"));
                assert!(seen.insert(index), "{at}: set twice");
                assert!((0.0..=1.0).contains(&value), "{at} = {value}");
                assert_eq!(module(t, preset).param(index), Some(value), "{at}");
            }
        }
    }
    // The resolver itself: a unique name, `#n`, and the names it refuses.
    let supersaw = module_spec(23).unwrap();
    assert_eq!(resolve(supersaw, "Filter Cutoff"), Some(19));
    assert_eq!(resolve(supersaw, "#12"), Some(12));
    assert_eq!(resolve(supersaw, "Post Gain"), Some(-1));
    assert_eq!(resolve(module_spec(0).unwrap(), "Post Gain"), Some(-1));
    // Shared or unknown names, Enable (param 0, which stays on), an empty name, and `#` without plain digits.
    for refused in [
        "Osc. Pulse Width",
        "",
        "Cutoff",
        "Enable",
        "#0",
        "#60",
        "#x",
        "#",
        "#+5",
        "#-1",
        "# 5",
    ] {
        assert_eq!(resolve(supersaw, refused), None, "{refused}");
    }
}

#[test]
fn names_are_unique_per_instrument_and_every_preset_has_a_name() {
    for (t, bank) in banks() {
        let mut names = HashSet::new();
        for preset in bank {
            assert!(!preset.name.trim().is_empty(), "{t}");
            assert!(names.insert(preset.name), "{t}: {} twice", preset.name);
        }
    }
}

#[test]
fn each_preset_reads_back_through_the_preset_loader_and_fits_its_module() {
    let mut read = 0;
    for (t, bank) in banks() {
        for preset in bank {
            let path = Path::new(ROOT).join(rel(t, preset).unwrap());
            let bytes = factory().read(&path).unwrap();
            assert!(preset_fits(&bytes, t), "{path:?}");
            let module = module(t, preset);
            assert_eq!(read_preset(&bytes, Some(t)).unwrap(), module, "{path:?}");
            // Named as a pick of it from the factory.
            let category = match preset.category {
                "" => format!("Presets/{}", preset_folder(t).unwrap()),
                c => format!("Presets/{}/{c}", preset_folder(t).unwrap()),
            };
            assert_eq!(module.file.category, category);
            assert_eq!(module.file.preset_name, preset.name);
            read += 1;
        }
    }
    assert_eq!(read, 12 + 11 + 8 + 14 + 10);
}

/// Each module folder lists its categories, or (Transistor Bass) its presets, as the picker's root shows them after
/// `My Files` and `Default`.
#[test]
fn each_module_folder_lists_its_categories_or_its_presets() {
    let rows = |folder: &str| -> Vec<(String, bool)> {
        let path = Path::new(ROOT).join(PRESETS).join(folder);
        let listed = factory().list(Some(&path)).unwrap();
        listed.into_iter().map(|e| (e.name, e.is_folder)).collect()
    };
    let folders = |names: &[&str]| -> Vec<(String, bool)> {
        names.iter().map(|n| (n.to_string(), true)).collect()
    };
    assert_eq!(rows("MiniSynth"), folders(&["Bass", "Keys", "Lead", "Pad"]));
    assert_eq!(rows("SuperSaw"), folders(&["Bass", "Lead", "Pad", "Pluck"]));
    assert_eq!(
        rows("GMSynth"),
        folders(&["Bass", "Bells", "Keys", "Lead", "Pad"])
    );
    assert_eq!(rows("3xOsc"), folders(&["Bass", "Lead", "Pad", "Synth"]));
    let tb = rows("TransistorBass");
    assert_eq!(tb.len(), 8);
    assert!(
        tb.iter()
            .all(|(name, folder)| !folder && name.ends_with(".flmpst"))
    );
}

/// A template's module ([`find`]) is the module a pick of the same preset gives, as the picker loads it
/// (`ui::views::preset_picker`: the factory's bytes through `read_preset`, `load_preset`, then `name_preset` from the
/// factory root), apart from the instance id its rack gave it.
#[test]
fn find_gives_the_module_a_pick_of_the_same_preset_gives() {
    let path = Path::new(ROOT).join("Presets/MiniSynth/Bass/Gravel Bass.flmpst");
    let bytes = factory().read(&path).unwrap();
    let mut picked = ModuleInstance::new(0, 7);
    picked.load_preset(read_preset(&bytes, Some(0)).unwrap());
    picked.name_preset(&path, Path::new(ROOT));
    // A pick keeps its source path relative to its root (`ProjectFiles::name_module`).
    picked.file.source_path = picked.file.preset_path.clone();
    let mut found = find(0, "Gravel Bass").expect("ours");
    assert_eq!(found.instance_id, 0);
    found.instance_id = 7;
    assert_eq!(found, picked);
}

#[test]
fn find_builds_a_preset_by_name_for_the_templates() {
    let preset = &bank(19)[0];
    assert_eq!(find(19, preset.name), Some(module(19, preset)));
    assert_eq!(find(19, "No Such Preset"), None);
    assert_eq!(find(24, preset.name), None);
}

#[test]
fn a_presets_settings_become_its_modules_cstm() {
    const WAVES: Preset = Preset {
        category: "Test",
        name: "Waves",
        params: &[],
        settings: &[(21100, 5.0), (21200, 17.0)],
    };
    let gm = module(2, &WAVES);
    let blob = gm.file.custom_data.as_deref().expect("a CSTM");
    let settings = settings_from_cstm(2, blob).unwrap();
    assert_eq!((settings[&21100], settings[&21200]), (5.0, 17.0));
    let bytes = flm::write_preset(&gm).unwrap();
    assert_eq!(read_preset(&bytes, Some(2)).unwrap(), gm);
    // No settings, no CSTM.
    assert_eq!(module(0, &bank(0)[0]).file.custom_data, None);
}

/// GMSynth's OSC A/B shapes live in its `CSTM`: each preset reads back from its factory file
/// with the slots it was given, inside the menu, and the bank plays more than a handful of them.
#[test]
fn gmsynth_presets_read_back_with_the_shapes_they_were_given() {
    let mut used = HashSet::new();
    for preset in bank(2) {
        let path = Path::new(ROOT).join(rel(2, preset).unwrap());
        let read = read_preset(&factory().read(&path).unwrap(), Some(2)).unwrap();
        let blob = read.file.custom_data.as_deref().expect("a CSTM");
        let settings = settings_from_cstm(2, blob).unwrap();
        let shapes = [settings[&21100], settings[&21200]];
        // Given as OSC A, then OSC B, and nothing else.
        let given = [(21100, shapes[0]), (21200, shapes[1])];
        assert_eq!(preset.settings, given, "{}", preset.name);
        for shape in shapes {
            assert!(
                shape.fract() == 0.0 && (0.0..29.0).contains(&shape),
                "{}",
                preset.name
            );
            used.insert(shape as u32);
        }
    }
    assert!(used.len() >= 12, "{used:?}");
}

/// 3x Osc's bank plays the fixed shapes only: no oscillator on `Custom` (round(6v) = 6, which needs a
/// `PATH`), and no `CSTM`.
#[test]
fn three_osc_presets_play_fixed_shapes_only() {
    for preset in bank(39) {
        let module = module(39, preset);
        for shape in 1..=3 {
            let v = module.param(shape).unwrap();
            assert!((v * 6.0).round() < 6.0, "{} osc {shape}: {v}", preset.name);
        }
        assert!(preset.settings.is_empty() && module.file.custom_data.is_none());
    }
}
