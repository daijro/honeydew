//! The patches of a SoundFont 2 file (the public SoundFont 2.x standard), for the SoundFont Player's patch selector:
//! `RIFF` `sfbk` → `LIST` `pdta` → `phdr`, 38-byte records. The parser is the audio engine's
//! (`honeydew_audio::soundfont`), which also plays the SoundFont: it checks the whole file and keeps the patches only, so
//! a SoundFont lists its patches exactly when the engine can play it, and one it cannot is an error here, which the
//! panel shows.

use std::fmt;

use honeydew_audio::soundfont::{ParseError, PresetHeader, preset_headers};
// The tests build files with these.
#[cfg(test)]
use honeydew_audio::soundfont::{MAX_PATCHES, PHDR_SIZE as RECORD};

/// One `phdr` record: a patch of the SoundFont.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    /// `achPresetName`, up to its first NUL.
    pub name: String,
    /// `wPreset`, the MIDI program.
    pub preset: u16,
    /// `wBank`.
    pub bank: u16,
}

/// Why a SoundFont's patches could not be listed. Its text is what the SoundFont Player's panel shows in its error
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sf2Error {
    /// The file could not be read: it is gone, or not a file.
    NotFound,
    /// The file is there but could not be read.
    Unreadable,
    /// The file is larger than the app reads ([`crate::project_files::MAX_FILE_BYTES`]).
    TooLarge,
    /// Not a `RIFF` `sfbk` file.
    NotSoundFont,
    /// `RIFF` `sfbk` file the engine cannot play: a chunk runs past its list (a file cut off in its sample data), a
    /// table or `sdta` `smpl` is missing, a table is not a whole number of records, the `phdr` has fewer than two (a
    /// patch and the terminal record), or a bag or generator index runs past its table or goes back.
    Corrupted,
}

impl fmt::Display for Sf2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "SoundFont file not found",
            Self::Unreadable => "Could not read SoundFont file",
            Self::TooLarge => "SoundFont file is too large",
            Self::NotSoundFont => "Not a SoundFont file",
            Self::Corrupted => "Corrupted SoundFont file",
        })
    }
}

/// The patches of the SoundFont `bytes`, in file order, without the terminal `EOP` record. The audio engine's parser
/// reads them ([`preset_headers`]: at most 65 536), and rejects what it could not play.
pub fn presets(bytes: &[u8]) -> Result<Vec<Preset>, Sf2Error> {
    let headers = preset_headers(bytes).map_err(|error| match error {
        ParseError::NotSoundFont => Sf2Error::NotSoundFont,
        ParseError::Corrupted => Sf2Error::Corrupted,
    })?;
    let preset = |header: &PresetHeader| Preset {
        name: crate::flm::chunk::text(header.name()),
        preset: header.preset,
        bank: header.bank,
    };
    Ok(headers.iter().map(preset).collect())
}

/// The patches of the file at `path`, `bytes`: a SoundFont's ([`presets`]), or for an `.sfz` one patch named after
/// the file. An SFZ is text that the engine reads with its samples; any that reads lists.
pub fn patches(path: &std::path::Path, bytes: &[u8]) -> Result<Vec<Preset>, Sf2Error> {
    if !crate::project_files::is_sfz(path) {
        return presets(bytes);
    }
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(vec![Preset {
        name,
        preset: 0,
        bank: 0,
    }])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A RIFF chunk: id, size, payload and the pad byte an odd size takes.
    fn chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = [&id[..], &(payload.len() as u32).to_le_bytes(), payload].concat();
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        chunk(b"LIST", &[&kind[..], &chunks.concat()].concat())
    }

    /// `phdr` record named `name` (cut at 20 bytes, NUL-padded).
    fn record(name: &str, preset: u16, bank: u16) -> Vec<u8> {
        let mut out = [0; RECORD];
        let name = &name.as_bytes()[..name.len().min(20)];
        out[..name.len()].copy_from_slice(name);
        out[20..22].copy_from_slice(&preset.to_le_bytes());
        out[22..24].copy_from_slice(&bank.to_le_bytes());
        out.to_vec()
    }

    fn riff(lists: &[Vec<u8>]) -> Vec<u8> {
        chunk(b"RIFF", &[&b"sfbk"[..], &lists.concat()].concat())
    }

    /// The bytes of `sdta`'s `smpl`.
    const SAMPLE_BYTES: usize = 400;

    /// The `INFO` list (with an odd-sized name) and the `sdta` list.
    fn info_and_samples() -> [Vec<u8>; 2] {
        [
            list(
                b"INFO",
                &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"INAM", b"Kit\0\0")],
            ),
            list(b"sdta", &[chunk(b"smpl", &[0; SAMPLE_BYTES])]),
        ]
    }

    /// `pdta` list with the `phdr` records `phdr` and every other table holding its terminal record only: no zones,
    /// instruments or samples, but whole.
    fn pdta(phdr: Vec<u8>) -> Vec<u8> {
        list(
            b"pdta",
            &[
                chunk(b"phdr", &phdr),
                chunk(b"pbag", &[0; 4]),
                chunk(b"pgen", &[0; 4]),
                chunk(b"inst", &[0; 22]),
                chunk(b"ibag", &[0; 4]),
                chunk(b"igen", &[0; 4]),
                chunk(b"shdr", &[0; 46]),
            ],
        )
    }

    /// The `phdr` records of `presets`, then `EOP`.
    fn phdr(presets: &[(&str, u16, u16)]) -> Vec<u8> {
        let mut phdr: Vec<u8> = presets
            .iter()
            .flat_map(|&(n, p, b)| record(n, p, b))
            .collect();
        phdr.extend(record("EOP", 255, 255));
        phdr
    }

    /// A small whole SoundFont with the patches `presets` (name, preset, bank), which the engine parses: `INFO`, `sdta`
    /// then `pdta`, as files lay them out.
    pub(crate) fn soundfont(presets: &[(&str, u16, u16)]) -> Vec<u8> {
        let [info, sdta] = info_and_samples();
        riff(&[info, sdta, pdta(phdr(presets))])
    }

    /// [`soundfont`] as a truncated download of a file that keeps `pdta` before `sdta`: every table is whole, so the
    /// patch headers read fine, but the file ends halfway through the sample data.
    pub(crate) fn cut_off_samples(presets: &[(&str, u16, u16)]) -> Vec<u8> {
        let [info, sdta] = info_and_samples();
        let mut bytes = riff(&[info, pdta(phdr(presets)), sdta]);
        bytes.truncate(bytes.len() - SAMPLE_BYTES / 2);
        bytes
    }

    #[test]
    fn the_patches_are_the_phdr_records_in_file_order_without_eop() {
        let bytes = soundfont(&[("Grand Piano", 0, 0), ("Strings", 48, 0), ("Drums", 0, 128)]);
        let got = presets(&bytes).unwrap();
        let want = [("Grand Piano", 0, 0), ("Strings", 48, 0), ("Drums", 0, 128)];
        let want: Vec<Preset> = want
            .iter()
            .map(|&(name, preset, bank)| Preset {
                name: name.into(),
                preset,
                bank,
            })
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_twenty_byte_name_has_no_nul_and_is_read_whole() {
        let bytes = soundfont(&[("Twenty Byte Name 123", 1, 2)]);
        assert_eq!(presets(&bytes).unwrap()[0].name, "Twenty Byte Name 123");
    }

    #[test]
    fn other_files_and_broken_soundfonts_are_errors() {
        use Sf2Error::*;
        let good = soundfont(&[("A", 0, 0)]);
        assert!(presets(&good).is_ok());
        assert_eq!(presets(b""), Err(NotSoundFont));
        assert_eq!(presets(b"RIFF\x04\0\0\0WAVE"), Err(NotSoundFont));
        assert_eq!(
            presets(&good[..good.len() - 20]),
            Err(Corrupted),
            "cut short"
        );
        // No pdta.
        assert_eq!(presets(&riff(&[list(b"sdta", &[])])), Err(Corrupted));
        // A pdta without a phdr.
        assert_eq!(
            presets(&riff(&[list(b"pdta", &[chunk(b"pbag", &[0; 4])])])),
            Err(Corrupted)
        );
        // A phdr that is not a whole number of records, or only the terminal one.
        let with = |bytes: Vec<u8>| riff(&[info_and_samples()[1].clone(), pdta(bytes)]);
        let mut odd = [record("A", 0, 0), record("EOP", 0, 0)].concat();
        odd.pop();
        assert_eq!(presets(&with(odd)), Err(Corrupted));
        assert_eq!(presets(&with(record("EOP", 0, 0))), Err(Corrupted));
        assert_eq!(presets(&with(Vec::new())), Err(Corrupted));
        // A chunk size past the end of its list.
        let mut huge = good.clone();
        let at = huge.windows(4).position(|w| w == b"phdr").unwrap();
        huge[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(presets(&huge), Err(Corrupted));
    }

    #[test]
    fn a_bank_with_whole_patch_headers_but_cut_off_samples_is_corrupted_not_listed() {
        // The headers read fine, but the engine cannot play the file, so it
        // must not list patches that play silence: the panel shows the error instead.
        use honeydew_audio::soundfont::SoundFont;
        let patches = [("A", 0, 0), ("B", 1, 0)];
        let whole = soundfont(&patches);
        assert!(SoundFont::parse(whole.clone()).is_ok());
        assert_eq!(presets(&whole).unwrap().len(), 2);
        let cut = cut_off_samples(&patches);
        assert!(SoundFont::parse(cut.clone()).is_err());
        assert_eq!(presets(&cut), Err(Sf2Error::Corrupted));
    }

    #[test]
    fn a_riff_size_past_the_end_reads_what_is_there() {
        let mut bytes = soundfont(&[("A", 0, 0)]);
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(presets(&bytes).unwrap().len(), 1);
    }

    #[test]
    fn at_most_65_536_patches_are_read() {
        let many = vec![("P", 0, 0); MAX_PATCHES + 2];
        assert_eq!(presets(&soundfont(&many)).unwrap().len(), MAX_PATCHES);
    }

    #[test]
    fn a_riff_size_shorter_than_its_form_type_holds_no_lists() {
        for size in 0..=4_u32 {
            let mut bytes = soundfont(&[("A", 0, 0)]);
            bytes[4..8].copy_from_slice(&size.to_le_bytes());
            assert_eq!(
                presets(&bytes),
                Err(Sf2Error::Corrupted),
                "RIFF size {size}"
            );
        }
    }
}
