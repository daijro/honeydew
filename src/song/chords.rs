//! Chord sets (`.chords` files, and the JSON `KEYB` holds): parsed from, and written back as, the same text.
//!
//! Details:
//! - Chord `i` goes to strip `i`, so extra chords are ignored. `parse` keeps the first [`STRIPS`] chords, so what a
//!   pick stores in `KEYB` is always 8 chords and always parses again, and refuses a set of fewer (`A chord set needs
//! 8 chords`). The keys in the file are exactly six; any other key is ignored, a missing one is an error.
//! - The ranges: `baseNote` 0..=11; `quality` 0..=6 (6 is power); every extension one of the seven with a name (9,
//!   10, 11, 13, 14, 17, 18), so a set with any other is refused. `octave` stays 0..=8 (the editor writes 2..=6, the
//!   bundled sets use 3..=5, and the default set is defined at 4). `rootNote` is any int: the key the set was written
//!   in, which the keyboard transposes from at use ([`ChordSet::transposed`]). A chord past the eighth is not checked.
//! - Extensions are a sorted set (duplicates collapse), so `parse` sorts and dedups them, and [`ChordSet::to_json`]
//!   writes them so.
//! - A file is read up to [`MAX_BYTES`] bytes; `parse` refuses a longer text rather than cut it short in the middle
//!   of its JSON. A leading UTF-8 BOM (an editor's) is skipped.

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// The chord strips of the chord keyboard; every bundled set has this many chords.
pub const STRIPS: usize = 8;
/// The most bytes of a `.chords` file the loader reads.
pub const MAX_BYTES: usize = 51_200;

/// `.chords` file: the chords in strip order, and the root note (`0` in every bundled set).
// Field order is the file's key order (alphabetical), which the derive writes in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChordSet {
    pub chords: Vec<Chord>,
    pub root_note: i32,
}

/// One chord strip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chord {
    /// The pitch class, 0 = C.
    pub base_note: i32,
    /// Extra intervals in semitones above the root ([`EXTENSIONS`]: 9 = 6th, 10 = m7, 11 = maj7, 13 = b9, 14 = 9th,
    /// 17 = 11th, 18 = #11).
    pub extensions: Vec<i32>,
    pub octave: i32,
    /// 0 major, 1 min, 2 aug, 3 dim, 4 Sus2, 5 Sus4, 6 power ([`QUALITIES`]).
    pub quality: i32,
}

impl ChordSet {
    /// The chord set in `json`, validated: a leading UTF-8 BOM is skipped, the first [`STRIPS`] chords are kept (the
    /// loader ignores the rest, which are not checked), and fewer than that is an error.
    pub fn parse(json: &str) -> Result<Self, String> {
        if json.len() > MAX_BYTES {
            return Err(format!("A chord set file is at most {MAX_BYTES} bytes."));
        }
        let json = json.strip_prefix('\u{feff}').unwrap_or(json);
        let mut set: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if set.chords.len() < STRIPS {
            return Err(format!("A chord set needs {STRIPS} chords"));
        }
        set.chords.truncate(STRIPS);
        for (i, chord) in set.chords.iter_mut().enumerate() {
            let at = |what: &str, value: i32, range: RangeInclusive<i32>| {
                range.contains(&value).then_some(()).ok_or_else(|| {
                    let (lo, hi) = (range.start(), range.end());
                    format!("Chord {}: {what} {value} is not in {lo}..={hi}", i + 1)
                })
            };
            at("baseNote", chord.base_note, 0..=11)?;
            at("octave", chord.octave, 0..=8)?;
            at("quality", chord.quality, 0..=6)?;
            if let Some(bad) = chord.extensions.iter().find(|e| !EXTENSIONS.contains(e)) {
                return Err(format!(
                    "Chord {}: extensions {bad} is not one of {EXTENSIONS:?}",
                    i + 1
                ));
            }
            chord.extensions.sort_unstable();
            chord.extensions.dedup();
        }
        Ok(set)
    }

    /// The set as `.chords` text: 4-space indent and keys `A-Z`, with no final newline.
    pub fn to_json(&self) -> String {
        let mut out = Vec::new();
        let format = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut writer = serde_json::Serializer::with_formatter(&mut out, format);
        self.serialize(&mut writer)
            .expect("a chord set always serializes");
        String::from_utf8(out).expect("JSON is UTF-8")
    }
}

/// The extensions a chord may have: each has a name.
pub const EXTENSIONS: [i32; 7] = [9, 10, 11, 13, 14, 17, 18];

/// Each quality's intervals in semitones: 0 major, 1 min, 2 aug, 3 dim, 4 Sus2, 5 Sus4, 6 power.
pub const QUALITIES: [[i32; 3]; 7] = [
    [0, 4, 7],
    [0, 3, 7],
    [0, 4, 8],
    [0, 3, 6],
    [0, 2, 7],
    [0, 5, 7],
    [0, 7, 12],
];

impl Chord {
    /// The voicing list L: the quality's intervals, then the extensions in ascending order; or, `compact` (Strum),
    /// their union with each extension taken mod 12, sorted.
    pub fn list(&self, compact: bool) -> Vec<i32> {
        let q = usize::try_from(self.quality)
            .unwrap_or(0)
            .min(QUALITIES.len() - 1);
        let mut list = QUALITIES[q].to_vec();
        let mut ext = self.extensions.clone();
        ext.sort_unstable();
        if compact {
            list.extend(ext.iter().map(|e| e.rem_euclid(12)));
            list.sort_unstable();
            list.dedup();
        } else {
            list.extend(ext);
        }
        list
    }

    /// The note at index `j` of `list` with the octave `shift`: 12·octave + baseNote + L[j mod N] + 12·floor(j/N) +
    /// 12·shift (no clamp).
    pub fn note(&self, list: &[i32], j: i32, shift: i32) -> i32 {
        let n = list.len() as i32;
        12 * self.octave
            + self.base_note
            + list[j.rem_euclid(n) as usize]
            + 12 * j.div_euclid(n)
            + 12 * shift
    }

    /// Inversion `k` (Chords mode): indices k … k + N − 1 of the list. −1 on a triad is the 5th an octave down, the
    /// root and the 3rd.
    pub fn inversion(&self, k: i32, shift: i32) -> Vec<i32> {
        let list = self.list(false);
        (k..k + list.len() as i32)
            .map(|j| self.note(&list, j, shift))
            .collect()
    }
}

/// The default chord set of a song whose scale has stored id `scale_id`, as KEYB keeps it (in C, root 0: for the major
/// scales the built-in set itself, [`crate::song::DEFAULT_CHORD_JSON`]), and its name.
pub fn default_keyb(scale_id: i32) -> (String, &'static str) {
    let minor = minor_default(scale_id);
    let json = ChordSet::default_set(minor).transposed(0).to_json();
    (json, if minor { "Minor" } else { "Major" })
}

/// The stored scale ids whose default chord set is `Minor`:
/// 7–14 (the `Minor …` scales), 19 Dorian, 25 Locrian and 30 Phrygian.
pub fn minor_default(scale_id: i32) -> bool {
    matches!(scale_id, 7..=14 | 19 | 25 | 30)
}

impl ChordSet {
    /// The set transposed to `key`: every chord by (key − rootNote) semitones, its
    /// root kept in 0…11 and its octave carrying, and the root note ← key.
    pub fn transposed(&self, key: i32) -> Self {
        let d = key - self.root_note;
        let chords = (self.chords.iter())
            .map(|c| {
                let n = 12 * c.octave + c.base_note + d;
                Chord {
                    base_note: n.rem_euclid(12),
                    octave: n.div_euclid(12),
                    ..c.clone()
                }
            })
            .collect();
        Self {
            chords,
            root_note: key,
        }
    }

    /// The built-in default set, `Major` or `Minor`, defined at octave 4 with root
    /// 12, so that in key C it plays an octave lower (C = MIDI 36):
    /// - Major: C, D min, E min, F, G, A min, B dim, then C an octave up;
    /// - Minor: C min, D dim, D#, F min, G min, G#, A#, then C min an octave up.
    pub fn default_set(minor: bool) -> Self {
        let major: [(i32, i32); 8] = [
            (0, 0),
            (2, 1),
            (4, 1),
            (5, 0),
            (7, 0),
            (9, 1),
            (11, 3),
            (0, 0),
        ];
        let minor_set: [(i32, i32); 8] = [
            (0, 1),
            (2, 3),
            (3, 0),
            (5, 1),
            (7, 1),
            (8, 0),
            (10, 0),
            (0, 1),
        ];
        let chords = (if minor { minor_set } else { major })
            .into_iter()
            .enumerate()
            .map(|(i, (base_note, quality))| Chord {
                base_note,
                extensions: Vec::new(),
                octave: 4 + i32::from(i == STRIPS - 1),
                quality,
            })
            .collect();
        Self {
            chords,
            root_note: 12,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::DEFAULT_CHORD_JSON;

    /// The default set's JSON with `field` of its first chord set to `value`.
    fn with(field: &str, value: serde_json::Value) -> String {
        let mut doc: serde_json::Value = serde_json::from_str(DEFAULT_CHORD_JSON).unwrap();
        doc["chords"][0][field] = value;
        doc.to_string()
    }

    #[test]
    fn the_default_set_reads_and_writes_back_byte_for_byte() {
        let set = ChordSet::parse(DEFAULT_CHORD_JSON).unwrap();
        assert_eq!((set.chords.len(), set.root_note), (STRIPS, 0));
        assert_eq!(set.to_json(), DEFAULT_CHORD_JSON);
    }

    #[test]
    fn a_set_writes_four_space_indent_with_keys_a_to_z_and_reads_back() {
        let chord = Chord {
            base_note: 7,
            extensions: vec![10, 14],
            octave: 4,
            quality: 1,
        };
        let set = ChordSet {
            chords: vec![chord; STRIPS],
            root_note: 0,
        };
        let json = set.to_json();
        let one = "        {\n            \"baseNote\": 7,\n            \"extensions\": [\n                10,\n                14\n            \
                   ],\n            \"octave\": 4,\n            \"quality\": 1\n        }";
        let expected = format!(
            "{{\n    \"chords\": [\n{}\n    ],\n    \"rootNote\": 0\n}}",
            [one; STRIPS].join(",\n")
        );
        assert_eq!(json, expected);
        assert_eq!(ChordSet::parse(&json).unwrap(), set);
    }

    #[test]
    fn each_range_is_rejected_just_outside_and_taken_at_its_bounds() {
        use serde_json::json;
        let bad = [
            ("baseNote", json!(-1)),
            ("baseNote", json!(12)),
            ("octave", json!(-1)),
            ("octave", json!(9)),
            ("quality", json!(-1)),
            ("quality", json!(7)),
            ("extensions", json!([0])),
            ("extensions", json!([9, 25])),
            ("extensions", json!([-3])),
            // Only the seven named extensions.
            ("extensions", json!([1])),
            ("extensions", json!([12])),
            ("extensions", json!([10, 15])),
            ("extensions", json!([24])),
        ];
        for (field, value) in bad {
            let err = ChordSet::parse(&with(field, value.clone())).unwrap_err();
            assert!(err.contains(field), "{field} {value}: {err}");
            assert!(err.contains("Chord 1"), "{err}");
        }
        let octave = ChordSet::parse(&with("octave", json!(9))).unwrap_err();
        assert_eq!(octave, "Chord 1: octave 9 is not in 0..=8");
        let good = [
            ("baseNote", json!(0)),
            ("baseNote", json!(11)),
            ("octave", json!(0)),
            ("octave", json!(8)),
            ("quality", json!(0)),
            ("quality", json!(6)),
            ("extensions", json!([9, 10, 11, 13, 14, 17, 18])),
            ("extensions", json!([])),
        ];
        for (field, value) in good {
            assert!(
                ChordSet::parse(&with(field, value.clone())).is_ok(),
                "{field} {value}"
            );
        }
        // The error names the chord: the third one here.
        let mut doc: serde_json::Value = serde_json::from_str(DEFAULT_CHORD_JSON).unwrap();
        doc["chords"][2]["octave"] = json!(40);
        assert!(
            ChordSet::parse(&doc.to_string())
                .unwrap_err()
                .contains("Chord 3")
        );
    }

    /// A set of `n` chords (the first one `first`, the rest the C major chord), as compact JSON.
    fn of(n: usize, first: &str) -> String {
        let rest = r#"{"baseNote":0,"extensions":[],"octave":3,"quality":0}"#;
        let chords: Vec<&str> = (0..n).map(|i| if i == 0 { first } else { rest }).collect();
        format!(r#"{{"chords":[{}],"rootNote":0}}"#, chords.join(","))
    }

    #[test]
    fn a_set_needs_eight_chords_and_every_key() {
        let one = r#"{"baseNote":0,"extensions":[],"octave":3,"quality":0}"#;
        assert!(ChordSet::parse(&of(8, one)).is_ok());
        for n in [0, 1, 7] {
            let err = ChordSet::parse(&of(n, one)).unwrap_err();
            assert_eq!(err, "A chord set needs 8 chords", "{n}");
        }
        for missing in ["baseNote", "extensions", "octave", "quality"] {
            let chord = one.replace(&format!("\"{missing}\""), "\"other\"");
            assert!(ChordSet::parse(&of(8, &chord)).is_err(), "{missing}");
        }
        let no_root = of(8, one).replace("\"rootNote\":0", "\"other\":0");
        assert!(ChordSet::parse(&no_root).is_err());
        for text in ["", "{", "[]", "null", "not json"] {
            assert!(ChordSet::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn chords_past_the_eighth_are_dropped_and_not_checked() {
        let one = r#"{"baseNote":0,"extensions":[],"octave":3,"quality":0}"#;
        let set = ChordSet::parse(&of(9, one)).unwrap();
        assert_eq!(set.chords.len(), STRIPS);
        // A bad value in chord 9 is ignored, as the loader ignores the chord.
        let mut doc: serde_json::Value = serde_json::from_str(&of(9, one)).unwrap();
        doc["chords"][8]["octave"] = serde_json::json!(40);
        doc["chords"][8]["extensions"] = serde_json::json!([0, 99]);
        let set = ChordSet::parse(&doc.to_string()).unwrap();
        assert_eq!(set.chords.len(), STRIPS);
        assert_eq!(set.chords[7].octave, 3);
        // What is stored (8 chords) reads again.
        assert_eq!(ChordSet::parse(&set.to_json()).unwrap(), set);
        // A file at the byte cap with hundreds of chords stores 8: a small text that reads again.
        let many = of(940, one);
        assert!((50_000..=MAX_BYTES).contains(&many.len()), "{}", many.len());
        let set = ChordSet::parse(&many).unwrap();
        assert_eq!(set.chords.len(), STRIPS);
        let stored = set.to_json();
        assert!(stored.len() < 2_000, "{}", stored.len());
        assert_eq!(ChordSet::parse(&stored).unwrap(), set);
    }

    #[test]
    fn a_leading_byte_order_mark_is_skipped() {
        let with_mark = format!("\u{feff}{DEFAULT_CHORD_JSON}");
        let set = ChordSet::parse(&with_mark).unwrap();
        assert_eq!(set.to_json(), DEFAULT_CHORD_JSON);
        // Only a leading one.
        let inside = DEFAULT_CHORD_JSON.replacen('{', "{\u{feff}", 1);
        assert!(ChordSet::parse(&inside).is_err());
    }

    #[test]
    fn extensions_are_a_sorted_set_and_other_keys_are_ignored() {
        let first = r#"{"quality": 1, "octave": 3, "baseNote": 5, "other": [], "extensions": [14, 10, 14, 11, 10]}"#;
        let json = of(8, first).replace(r#""rootNote":0"#, r#""extra": 1, "rootNote": 2"#);
        let set = ChordSet::parse(&json).unwrap();
        assert_eq!(set.chords[0].extensions, [10, 11, 14]);
        assert_eq!(set.root_note, 2);
        // What `to_json` writes has neither the duplicates nor the other keys.
        let back = ChordSet::parse(&set.to_json()).unwrap();
        assert_eq!(back, set);
        assert!(!set.to_json().contains("other"));
    }

    #[test]
    fn a_file_over_the_loaders_limit_is_refused() {
        let json = DEFAULT_CHORD_JSON;
        let padded = |len: usize| format!("{json}{}", " ".repeat(len - json.len()));
        // Exactly at the limit is read; one byte more is not.
        assert!(ChordSet::parse(&padded(MAX_BYTES)).is_ok());
        let err = ChordSet::parse(&padded(MAX_BYTES + 1)).unwrap_err();
        assert!(err.contains("51200"), "{err}");
        assert!(ChordSet::parse(&padded(2 * MAX_BYTES)).is_err());
    }

    fn chord(base_note: i32, quality: i32, extensions: &[i32]) -> Chord {
        Chord {
            base_note,
            extensions: extensions.to_vec(),
            octave: 4,
            quality,
        }
    }

    #[test]
    fn each_quality_has_its_intervals_then_the_extensions_and_strum_folds_them_into_the_octave() {
        let lists: Vec<Vec<i32>> = (0..7).map(|q| chord(0, q, &[]).list(false)).collect();
        assert_eq!(
            lists,
            [
                [0, 4, 7],
                [0, 3, 7],
                [0, 4, 8],
                [0, 3, 6],
                [0, 2, 7],
                [0, 5, 7],
                [0, 7, 12]
            ]
        );
        // Chords: the extensions after the quality, ascending; Strum: the union with each extension mod 12, sorted.
        let c = chord(0, 1, &[14, 10]);
        assert_eq!(c.list(false), [0, 3, 7, 10, 14]);
        assert_eq!(c.list(true), [0, 2, 3, 7, 10]);
        assert_eq!(chord(0, 6, &[14]).list(true), [0, 2, 7, 12]);
    }

    #[test]
    fn notes_wrap_up_an_octave_past_the_list_and_inversions_take_n_indices_from_k() {
        // C major at octave 4: C = 48.
        let c = chord(0, 0, &[]);
        let list = c.list(false);
        assert_eq!(c.note(&list, 0, 0), 48);
        assert_eq!(c.note(&list, 3, 0), 60);
        assert_eq!(c.note(&list, -1, 0), 43, "the 5th an octave down");
        assert_eq!(c.note(&list, 0, -2), 24, "the octave shift");
        assert_eq!(c.inversion(0, 0), [48, 52, 55]);
        assert_eq!(c.inversion(-1, 0), [43, 48, 52]);
        assert_eq!(c.inversion(-2, 0), [40, 43, 48]);
        assert_eq!(c.inversion(3, 1), [72, 76, 79]);
        // Four notes: inversion 1 of a 7th chord.
        assert_eq!(chord(7, 0, &[10]).inversion(1, 0), [59, 62, 65, 67]);
    }

    #[test]
    fn a_set_transposes_by_the_key_minus_its_root_with_octaves_carrying() {
        let set = ChordSet::default_set(false);
        assert_eq!((set.root_note, set.chords[0].octave), (12, 4));
        // In C: an octave down, root 0.
        let c = set.transposed(0);
        assert_eq!(c.root_note, 0);
        assert_eq!(
            c.chords
                .iter()
                .map(|c| (c.base_note, c.octave))
                .collect::<Vec<_>>(),
            [
                (0, 3),
                (2, 3),
                (4, 3),
                (5, 3),
                (7, 3),
                (9, 3),
                (11, 3),
                (0, 4)
            ]
        );
        // The default Major set in C plays C = MIDI 36.
        assert_eq!(c.chords[0].inversion(0, 0), [36, 40, 43]);
        assert_eq!(c.to_json(), crate::song::DEFAULT_CHORD_JSON);
        // In A: B dim goes to G# (base 8) of octave 4; the last C to A, still octave 4.
        let a = set.transposed(9);
        assert_eq!((a.chords[6].base_note, a.chords[6].octave), (8, 4));
        assert_eq!((a.chords[7].base_note, a.chords[7].octave), (9, 4));
        // Down across octave 0 and back.
        assert_eq!(a.transposed(0), c);
    }

    #[test]
    fn the_minor_default_is_c_min_d_dim_d_sharp_f_min_g_min_g_sharp_a_sharp_c_min() {
        let set = ChordSet::default_set(true).transposed(0);
        let q: Vec<(i32, i32)> = set
            .chords
            .iter()
            .map(|c| (c.base_note, c.quality))
            .collect();
        assert_eq!(
            q,
            [
                (0, 1),
                (2, 3),
                (3, 0),
                (5, 1),
                (7, 1),
                (8, 0),
                (10, 0),
                (0, 1)
            ]
        );
        assert_eq!(set.chords[7].octave, 4);
        // Minor for ids 7–14, 19, 25 and 30 of the stored scale list: the `Minor …` names, Dorian, Locrian, Phrygian.
        use crate::song::scales::{SCALES, scale_from_stored};
        let minor: Vec<&str> = (0..40)
            .filter(|&id| minor_default(id))
            .map(|id| SCALES[scale_from_stored(id)])
            .collect();
        assert_eq!(minor.len(), 11);
        let named =
            |n: &&str| n.starts_with("Minor") || ["Dorian", "Locrian", "Phrygian"].contains(n);
        assert!(minor.iter().all(named), "{minor:?}");
        for id in [0, 1, 6, 15, 26, 31, 32, -1, 40] {
            assert!(!minor_default(id), "{id}");
        }
    }

    #[test]
    fn a_power_chord_and_every_named_extension_read_and_a_set_in_another_key_transposes_at_use() {
        let mut doc: serde_json::Value = serde_json::from_str(DEFAULT_CHORD_JSON).unwrap();
        doc["chords"][0]["quality"] = serde_json::json!(6);
        doc["chords"][1]["extensions"] = serde_json::json!([18, 9, 13]);
        doc["rootNote"] = serde_json::json!(5);
        let set = ChordSet::parse(&doc.to_string()).unwrap();
        assert_eq!(set.chords[0].list(false), [0, 7, 12]);
        assert_eq!(set.chords[1].extensions, [9, 13, 18]);
        // Written in F (5): in C it plays 5 semitones lower, octaves carrying.
        let c = set.transposed(0);
        assert_eq!(c.root_note, 0);
        assert_eq!((c.chords[0].base_note, c.chords[0].octave), (7, 2));
        assert_eq!((c.chords[1].base_note, c.chords[1].octave), (9, 2));
        let err = ChordSet::parse(&with("extensions", serde_json::json!([16]))).unwrap_err();
        assert_eq!(
            err,
            "Chord 1: extensions 16 is not one of [9, 10, 11, 13, 14, 17, 18]"
        );
    }
}
