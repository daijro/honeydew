//! Our chord sets, `Chord Sets/<Name>.chords`: eight progressions of our own in `assets/content/chord-sets/`, written
//! in the `.chords` layout and embedded as text. A set is eight chords in strip order, `rootNote` 0, in the key of C,
//! with octaves 3-5, qualities 0 (major), 1 (minor) and 3 (diminished) only, and extensions from 9, 10, 11, 14 and 17.
//! Each is read back by [`crate::song::chords::ChordSet::parse`].
//!
//! They list in index order, the bytes of the file names (a factory folder is not re-sorted); for these names that is
//! also `A-Z`.

use super::{CHORD_SETS, Entry};

/// The sets: a name, and the file's text.
const SETS: [(&str, &str); 8] = [
    (
        "Dark Minor",
        include_str!("../../assets/content/chord-sets/Dark Minor.chords"),
    ),
    (
        "Dorian Walk",
        include_str!("../../assets/content/chord-sets/Dorian Walk.chords"),
    ),
    (
        "Epic Rise",
        include_str!("../../assets/content/chord-sets/Epic Rise.chords"),
    ),
    (
        "Gospel Nines",
        include_str!("../../assets/content/chord-sets/Gospel Nines.chords"),
    ),
    (
        "Lo-Fi Haze",
        include_str!("../../assets/content/chord-sets/Lo-Fi Haze.chords"),
    ),
    (
        "Night Minor",
        include_str!("../../assets/content/chord-sets/Night Minor.chords"),
    ),
    (
        "Pop Circle",
        include_str!("../../assets/content/chord-sets/Pop Circle.chords"),
    ),
    (
        "Soul Sevenths",
        include_str!("../../assets/content/chord-sets/Soul Sevenths.chords"),
    ),
];

pub(super) fn entries() -> Vec<Entry> {
    let entry = |&(name, json): &(&str, &'static str)| {
        Entry::new(format!("{CHORD_SETS}/{name}.chords"), move || {
            json.as_bytes().to_vec()
        })
    };
    SETS.iter().map(entry).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::content::{CHORD_SETS, ROOT, factory};
    use crate::song::chords::{ChordSet, STRIPS};
    use crate::ui::main_menu::SongLibrary;

    /// The set names, in the order the factory lists them: the bytes of their paths, which is also `A-Z` for these.
    const NAMES: [&str; 8] = [
        "Dark Minor",
        "Dorian Walk",
        "Epic Rise",
        "Gospel Nines",
        "Lo-Fi Haze",
        "Night Minor",
        "Pop Circle",
        "Soul Sevenths",
    ];

    fn folder() -> std::path::PathBuf {
        Path::new(ROOT).join(CHORD_SETS)
    }

    fn texts() -> Vec<(String, String)> {
        let library = factory();
        let rows = library.list(Some(&folder())).unwrap();
        let read = |path: &Path| String::from_utf8(library.read(path).unwrap()).unwrap();
        rows.into_iter()
            .map(|row| (row.name, read(&row.path)))
            .collect()
    }

    #[test]
    fn the_factory_lists_eight_sets_in_index_order() {
        let rows = factory().list(Some(&folder())).unwrap();
        let listed: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        let files: Vec<String> = NAMES.iter().map(|n| format!("{n}.chords")).collect();
        assert_eq!(listed, files);
        assert!(rows.iter().all(|r| !r.is_folder));
        // Sorted by the bytes of the file names, as a factory bank is ordered.
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(sorted, files);
    }

    #[test]
    fn every_factory_file_parses_and_is_already_in_the_written_form() {
        for (name, text) in texts() {
            let set = ChordSet::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(set.to_json(), text, "{name}");
        }
    }

    #[test]
    fn every_set_is_eight_chords_in_the_plans_vocabulary() {
        for (name, text) in texts() {
            let set = ChordSet::parse(&text).unwrap();
            assert_eq!((set.chords.len(), set.root_note), (STRIPS, 0), "{name}");
            for chord in &set.chords {
                assert!((3..=5).contains(&chord.octave), "{name}: {chord:?}");
                assert!([0, 1, 3].contains(&chord.quality), "{name}: {chord:?}");
                let allowed = [9, 10, 11, 14, 17];
                assert!(
                    chord.extensions.iter().all(|e| allowed.contains(e)),
                    "{name}: {chord:?}"
                );
            }
        }
    }

    #[test]
    fn the_sets_are_distinct_and_none_is_the_default_one() {
        let all = texts();
        for (i, (a, ta)) in all.iter().enumerate() {
            assert_ne!(ta, crate::song::DEFAULT_CHORD_JSON, "{a}");
            for (b, tb) in &all[i + 1..] {
                assert_ne!(ta, tb, "{a} = {b}");
            }
        }
    }
}
