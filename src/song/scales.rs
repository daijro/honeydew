//! The project scales: the key lock's scale list (`Song::key_scale`) with each scale's tones, shared by the keyboard
//! settings, the piano roll's scale tools and the Scale (50) effect panel.

/// Every scale, in the keyboard settings' `Project Scale` picker order (read column by column), which is also the
/// Scale (50) effect's option order (less `Custom`). A scale's id is its index here.
pub const SCALES: [&str; 33] = [
    // Major
    "Major",
    "Major Bebop",
    "Major Bulgarian",
    "Major Pentatonic",
    "Major Persian",
    "Major Polymode",
    "Lydian",
    "Mixolydian",
    "Phrygian Dominant",
    // Minor
    "Minor Harmonic",
    "Minor Hungarian",
    "Minor Melodic",
    "Minor Natural",
    "Minor Neapolitan",
    "Minor Pentatonic",
    "Minor Polymode",
    "Minor Romanian",
    "Dorian",
    "Phrygian",
    "Locrian",
    // Other
    "Chromatic",
    "Arabic",
    "Blues",
    "Diminished",
    "Dominant Bebop",
    "Egyptian",
    "Enigmatic",
    "Hirajoshi",
    "Iwato",
    "Japanese Insen",
    "Locrian Super",
    "Neapolitan",
    "Whole Tone",
];
/// The default scale's id.
pub const CHROMATIC: usize = 20;
/// Each [`SCALES`] entry's tones, C…B relative to the key, most significant bit C (1 = in scale). `panels::fx::scale`
/// tests the table.
pub const SCALE_MASKS: [u16; 33] = [
    // Major
    0b101011010101,
    0b101011011101,
    0b101001101101,
    0b101010010100,
    0b110011101001,
    0b101101101011,
    0b101010110101,
    0b101011010110,
    0b110011011010,
    // Minor
    0b101101011001,
    0b111100111001,
    0b101101010101,
    0b101101011010,
    0b110101011001,
    0b100101010010,
    0b111011011010,
    0b101100110110,
    0b101101010110,
    0b110101011010,
    0b110101101010,
    // Other
    0b111111111111,
    0b110011011001,
    0b100101110010,
    0b101101101101,
    0b101011010111,
    0b101001010010,
    0b110010101011,
    0b101100011000,
    0b110001100010,
    0b110001010010,
    0b110110101010,
    0b110101010101,
    0b101010101010,
];

/// The song file's key-lock scale ids (HEAD@348): Chromatic first, then the names starting `Major` (1–6) and `Minor`
/// (7–14), then the rest A to Z (15–32). The picker shows the same scales in its own columns ([`SCALES`]).
const STORED_SCALES: [&str; 33] = [
    "Chromatic",
    "Major",
    "Major Bebop",
    "Major Bulgarian",
    "Major Pentatonic",
    "Major Persian",
    "Major Polymode",
    "Minor Harmonic",
    "Minor Hungarian",
    "Minor Melodic",
    "Minor Natural",
    "Minor Neapolitan",
    "Minor Pentatonic",
    "Minor Polymode",
    "Minor Romanian",
    "Arabic",
    "Blues",
    "Diminished",
    "Dominant Bebop",
    "Dorian",
    "Egyptian",
    "Enigmatic",
    "Hirajoshi",
    "Iwato",
    "Japanese Insen",
    "Locrian",
    "Locrian Super",
    "Lydian",
    "Mixolydian",
    "Neapolitan",
    "Phrygian",
    "Phrygian Dominant",
    "Whole Tone",
];

/// The [`SCALES`] index of a stored key-lock scale id ([`super::Song::key_scale`]). Ids outside the list read as
/// `Chromatic`.
pub fn scale_from_stored(id: i32) -> usize {
    usize::try_from(id)
        .ok()
        .and_then(|i| STORED_SCALES.get(i))
        .and_then(|name| SCALES.iter().position(|s| s == name))
        .unwrap_or(CHROMATIC)
}

/// The stored key-lock scale id of [`SCALES`] index `index` (`Chromatic`'s, 0, past the end).
pub fn scale_to_stored(index: usize) -> i32 {
    let name = SCALES.get(index).copied().unwrap_or("Chromatic");
    STORED_SCALES
        .iter()
        .position(|&s| s == name)
        .map_or(0, |i| i as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_scale_ids_are_the_round_23_table() {
        // The stored scale ids: Chromatic, the `Major …` names, the `Minor …` names, then the rest A to Z.
        let names: Vec<&str> = (0..33).map(|id| SCALES[scale_from_stored(id)]).collect();
        assert_eq!(
            names,
            [
                "Chromatic",
                "Major",
                "Major Bebop",
                "Major Bulgarian",
                "Major Pentatonic",
                "Major Persian",
                "Major Polymode",
                "Minor Harmonic",
                "Minor Hungarian",
                "Minor Melodic",
                "Minor Natural",
                "Minor Neapolitan",
                "Minor Pentatonic",
                "Minor Polymode",
                "Minor Romanian",
                "Arabic",
                "Blues",
                "Diminished",
                "Dominant Bebop",
                "Dorian",
                "Egyptian",
                "Enigmatic",
                "Hirajoshi",
                "Iwato",
                "Japanese Insen",
                "Locrian",
                "Locrian Super",
                "Lydian",
                "Mixolydian",
                "Neapolitan",
                "Phrygian",
                "Phrygian Dominant",
                "Whole Tone"
            ]
        );
        // The file's default, 0, is the picker's default, Chromatic; the quick buttons' ids.
        assert_eq!(scale_from_stored(0), CHROMATIC);
        assert_eq!(scale_to_stored(CHROMATIC), 0);
        for (id, name) in [
            (1, "Major"),
            (7, "Minor Harmonic"),
            (9, "Minor Melodic"),
            (10, "Minor Natural"),
        ] {
            assert_eq!(SCALES[scale_from_stored(id)], name);
        }
        assert_eq!(SCALES[scale_from_stored(3)], "Major Bulgarian");
        assert_eq!(SCALES[scale_from_stored(32)], "Whole Tone");
        assert_ne!(scale_from_stored(2), CHROMATIC);
        assert_ne!(scale_from_stored(8), CHROMATIC);
        // Every picker scale has an id, and the ids map back.
        for (index, name) in SCALES.iter().enumerate() {
            assert_eq!(scale_from_stored(scale_to_stored(index)), index, "{name}");
        }
        // Negative ids and ids past the list read as Chromatic.
        for id in [-1, 33, 34, 35, i32::MAX] {
            assert_eq!(scale_from_stored(id), CHROMATIC, "{id}");
        }
    }
}
