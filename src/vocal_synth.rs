//! Voicebank metadata and UTAU `oto.ini` support for BanaForge's Vocal Synth.
//!
//! Voicebank files are user content: this module only indexes metadata and never bundles or republishes samples. A
//! bank's own license must be checked before sharing a project that depends on it.

pub mod preparation;

use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

/// One UTAU `oto.ini` alias and its timing values, in milliseconds.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OtoEntry {
    /// Relative WAV path, as written in `oto.ini`.
    pub filename: String,
    pub alias: String,
    pub offset: f64,
    pub consonant: f64,
    pub cutoff: f64,
    pub preutterance: f64,
    pub overlap: f64,
}

/// A bank's searchable, license-neutral index. The samples remain in the user's imported archive.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VoicebankIndex {
    pub name: String,
    pub oto_path: String,
    /// Decoded archive root for resolving subbank-relative WAV filenames.
    #[serde(default)]
    pub root: String,
    pub entries: Vec<OtoEntry>,
    /// A license/readme file was found; its contents are deliberately not interpreted as a license grant.
    pub has_license_document: bool,
}

/// UTAU text is Windows-31J unless a Unicode BOM explicitly identifies it.
/// `encoding_rs::SHIFT_JIS` implements the WHATWG Windows-31J/CP932 mapping.
pub fn decode_metadata(data: &[u8]) -> Result<std::borrow::Cow<'_, str>, String> {
    let (encoding, skip) = if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        (encoding_rs::UTF_8, 3)
    } else if data.starts_with(&[0xff, 0xfe]) {
        (encoding_rs::UTF_16LE, 2)
    } else if data.starts_with(&[0xfe, 0xff]) {
        (encoding_rs::UTF_16BE, 2)
    } else {
        (encoding_rs::SHIFT_JIS, 0)
    };
    let (text, invalid) = encoding.decode_without_bom_handling(&data[skip..]);
    if invalid {
        return Err(
            "Invalid voicebank text encoding (expected Windows-31J or BOM-marked Unicode)".into(),
        );
    }
    Ok(text)
}

fn character_name(files: &[crate::zip::Entry], root: &str) -> Option<String> {
    files
        .iter()
        .filter(|file| {
            let (dir, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
            dir == root && name.eq_ignore_ascii_case("character.txt")
        })
        .find_map(|file| {
            let text = decode_metadata(&file.data).ok()?;
            text.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                let value = value.trim();
                (key.trim().eq_ignore_ascii_case("name")
                    && !value.is_empty()
                    && !value.chars().any(char::is_control))
                .then(|| value.nfc().collect())
            })
        })
}

/// License presence is not a permission grant. Do not infer rights from its text.
pub fn license_document(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    basename.starts_with("license")
        || basename.starts_with("licence")
        || basename.contains("使用許諾")
        || basename.contains("利用規約")
        || basename.contains("利用条件")
        || basename.contains("使用条件")
}

impl VoicebankIndex {
    /// Resolve a WAV with the same NFC policy as decoded ZIP paths. No decoding/audio I/O.
    pub fn matching_sample<'a>(
        &self,
        files: &'a [crate::zip::Entry],
        alias: &OtoEntry,
    ) -> Option<&'a crate::zip::Entry> {
        let path = if self.root.is_empty() {
            alias.filename.clone()
        } else {
            format!("{}/{}", self.root, alias.filename)
        };
        let path: String = path.nfc().collect();
        files.iter().find(|file| file.path.nfc().eq(path.chars()))
    }
}

/// Parse a UTAU `oto.ini` into a stable alias-sorted list.
///
/// The alias parsing follows the MIT-licensed WebTAU UTAU metadata parser, adapted for BanaForge's safe relative-path
/// checks and Shift-JIS decoding. See `THIRD_PARTY_NOTICES.md` and `licenses/webtau-MIT.txt`.
pub fn parse_oto_ini(data: &[u8]) -> Result<Vec<OtoEntry>, String> {
    let decoded = decode_metadata(data)?;
    let mut entries = BTreeMap::<String, OtoEntry>::new();

    for line in decoded.lines() {
        let line = line.trim().trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((filename, values)) = line.split_once('=') else {
            continue;
        };
        let filename: String = filename.trim().replace('\\', "/").nfc().collect();
        if !safe_sample_path(&filename) {
            continue;
        }
        let mut values = values.split(',');
        let raw_alias = values.next().unwrap_or("").trim();
        let alias = if raw_alias.is_empty() {
            filename
                .rsplit('/')
                .next()
                .unwrap_or(&filename)
                .rsplit_once('.')
                .filter(|(_, ext)| ext.eq_ignore_ascii_case("wav"))
                .map_or_else(|| filename.clone(), |(stem, _)| stem.to_owned())
        } else {
            raw_alias.nfc().collect()
        };
        let values: Vec<f64> = values
            .take(5)
            .map(|value| value.trim().parse::<f64>().unwrap_or(f64::NAN))
            .collect();
        if values.len() < 5 || values.iter().any(|value| !value.is_finite()) || alias.is_empty() {
            continue;
        }
        entries.insert(
            alias.clone(),
            OtoEntry {
                filename,
                alias,
                offset: values[0],
                consonant: values[1],
                cutoff: values[2],
                preutterance: values[3],
                overlap: values[4],
            },
        );
    }

    if entries.is_empty() {
        return Err("No valid oto.ini entries found".into());
    }
    Ok(entries.into_values().collect())
}

/// Parse all `oto.ini` files in a `.zip`/`.uar` voicebank using BanaForge's bounded ZIP reader.
///
/// This intentionally indexes metadata only; samples are not extracted or embedded in the project.
pub fn index_archive(bytes: &[u8], max_unpacked: u64) -> Result<VoicebankIndex, String> {
    let files = crate::zip::read_voicebank(bytes, max_unpacked).map_err(|err| err.to_string())?;
    index_files(&files)
}

/// Import entry point: a bank without `character.txt` uses the user's archive stem,
/// never a decoded internal ZIP directory as its displayed name.
pub fn index_archive_named(
    bytes: &[u8],
    max_unpacked: u64,
    fallback: &str,
) -> Result<VoicebankIndex, String> {
    let files = crate::zip::read_voicebank(bytes, max_unpacked).map_err(|err| err.to_string())?;
    index_files_named(&files, Some(fallback))
}

/// Index all pitch/subbank oto files, resolving WAV paths relative to the selected bank root.
/// No sample bytes are retained in the manifest.
pub fn index_files(files: &[crate::zip::Entry]) -> Result<VoicebankIndex, String> {
    index_files_named(files, None)
}

fn index_files_named(
    files: &[crate::zip::Entry],
    fallback: Option<&str>,
) -> Result<VoicebankIndex, String> {
    let oto = files
        .iter()
        .filter(|entry| {
            entry
                .path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("oto.ini"))
        })
        .min_by(|a, b| {
            a.path
                .matches('/')
                .count()
                .cmp(&b.path.matches('/').count())
                .then(a.path.cmp(&b.path))
        })
        .ok_or_else(|| "This archive does not contain an oto.ini voicebank index".to_owned())?;
    let mut root = oto.path.rsplit_once('/').map_or("", |(root, _)| root);
    // Many multi-pitch banks have no root oto, only A3/oto.ini, C4/oto.ini, etc.
    // character.txt is the bank marker; otherwise use the shared ancestor when all otos
    // belong to the same top-level folder. Never merge unrelated top-level banks.
    if let Some(marker) = files
        .iter()
        .filter(|file| {
            file.path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("character.txt"))
        })
        .find_map(|file| {
            let dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
            (dir.is_empty()
                || root == dir
                || root.strip_prefix(dir).is_some_and(|p| p.starts_with('/')))
            .then_some(dir)
        })
    {
        root = marker;
    } else {
        let top = root.split('/').next().unwrap_or("");
        let otos: Vec<_> = files
            .iter()
            .filter(|file| {
                file.path
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case("oto.ini"))
            })
            .collect();
        if !top.is_empty()
            && otos
                .iter()
                .all(|file| file.path.split('/').next() == Some(top))
        {
            for file in otos {
                let dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
                while root != dir && !dir.strip_prefix(root).is_some_and(|p| p.starts_with('/')) {
                    root = root.rsplit_once('/').map_or("", |(parent, _)| parent);
                    if root.is_empty() {
                        break;
                    }
                }
            }
        }
    }
    let mut entries = Vec::new();
    for file in files.iter().filter(|file| {
        file.path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("oto.ini"))
    }) {
        let dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
        let relative = if root.is_empty() {
            dir
        } else if dir == root {
            ""
        } else if let Some(relative) = dir.strip_prefix(root).and_then(|p| p.strip_prefix('/')) {
            relative
        } else {
            continue;
        };
        // A malformed subbank must not hide valid aliases from the other pitch folders.
        if let Ok(mut aliases) = parse_oto_ini(&file.data) {
            for alias in &mut aliases {
                if !relative.is_empty() {
                    alias.filename = format!("{relative}/{}", alias.filename);
                }
            }
            entries.extend(aliases);
        }
    }
    if entries.is_empty() {
        return Err("No valid oto.ini entries found".into());
    }
    entries.sort_by(|a, b| a.alias.cmp(&b.alias).then(a.filename.cmp(&b.filename)));
    let name = character_name(files, root).unwrap_or_else(|| {
        fallback
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                root.rsplit('/')
                    .next()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Voicebank")
            })
            .nfc()
            .collect()
    });
    let has_license_document = files.iter().any(|entry| {
        let in_root = root.is_empty()
            || entry
                .path
                .strip_prefix(root)
                .is_some_and(|p| p.starts_with('/'));
        in_root && license_document(&entry.path)
    });
    Ok(VoicebankIndex {
        name,
        oto_path: oto.path.clone(),
        root: root.nfc().collect(),
        entries,
        has_license_document,
    })
}

fn safe_sample_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocal_metadata_uses_cp932_unless_a_unicode_bom_is_present() {
        // These bytes are valid UTF-8 too: UTAU metadata must still use CP932.
        assert_eq!(decode_metadata(&[0xc2, 0xa5]).unwrap(), "ﾂ･");
        assert_eq!(
            decode_metadata("\u{feff}name=が".as_bytes()).unwrap(),
            "name=が"
        );
        for (bom, little) in [([0xff, 0xfe], true), ([0xfe, 0xff], false)] {
            let mut bytes = bom.to_vec();
            for unit in "使用許諾条件".encode_utf16() {
                bytes.extend_from_slice(&if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            assert_eq!(decode_metadata(&bytes).unwrap(), "使用許諾条件");
        }
        assert!(decode_metadata(&[0xff, 0xfe, 0x41]).is_err());
    }

    #[test]
    fn vocal_bank_without_character_name_uses_archive_stem_not_internal_directory() {
        let zip = crate::zip::write(&[("Internal/oto.ini".into(), b"a.wav=a,0,0,0,0,0".to_vec())])
            .unwrap();
        assert_eq!(
            index_archive_named(&zip, 1024, "User Bank").unwrap().name,
            "User Bank"
        );
    }

    #[test]
    fn decodes_real_shift_jis_and_rejects_broken_encoding() {
        let (bytes, _, invalid) = encoding_rs::SHIFT_JIS.encode("あ.wav=あ,0,1,2,3,4");
        assert!(!invalid);
        let entries = parse_oto_ini(&bytes).unwrap();
        assert_eq!(entries[0].alias, "あ");
        assert_eq!(entries[0].filename, "あ.wav");
        assert!(parse_oto_ini(b"a.wav=a,0,1,2,3,4\n\x81").is_err());
    }

    #[test]
    fn finds_multi_pitch_bank_without_a_root_oto() {
        let files = vec![
            crate::zip::Entry {
                path: "Bank/A3/oto.ini".into(),
                data: b"a.wav=a_A3,0,0,0,0,0".to_vec(),
            },
            crate::zip::Entry {
                path: "Bank/C4/oto.ini".into(),
                data: b"a.wav=a_C4,0,0,0,0,0".to_vec(),
            },
        ];
        let index = index_files(&files).unwrap();
        assert_eq!(index.name, "Bank");
        assert_eq!(index.entries.len(), 2);
        assert_eq!(index.entries[1].filename, "C4/a.wav");
    }

    #[test]
    fn rejects_nonfinite_timings_and_control_characters() {
        for bad in ["NaN", "inf", "-inf", "broken"] {
            assert!(parse_oto_ini(format!("a.wav=a,{bad},0,0,0,0").as_bytes()).is_err());
        }
        assert!(parse_oto_ini(b"a\0.wav=a,0,0,0,0,0").is_err());
        // Negative cutoff is valid UTAU fixed-length notation.
        assert!(parse_oto_ini(b"a.wav=a,0,0,-100,0,0").is_ok());
    }

    #[test]
    fn indexes_pitch_folders_and_resolves_their_samples() {
        let files = vec![
            crate::zip::Entry {
                path: "Bank/A3/oto.ini".into(),
                data: b"a.wav=a_A3,0,0,0,0,0".to_vec(),
            },
            crate::zip::Entry {
                path: "Bank/oto.ini".into(),
                data: Vec::new(),
            },
            crate::zip::Entry {
                path: "Bank/C4/oto.ini".into(),
                data: b"a.wav=a_C4,0,0,0,0,0".to_vec(),
            },
            crate::zip::Entry {
                path: "Other/oto.ini".into(),
                data: b"b.wav=b,0,0,0,0,0".to_vec(),
            },
        ];
        let index = index_files(&files).unwrap();
        assert_eq!(index.entries.len(), 2);
        assert_eq!(index.entries[0].filename, "A3/a.wav");
        assert_eq!(index.entries[1].filename, "C4/a.wav");
    }

    #[test]
    fn parses_utf8_shift_jis_bom_comments_and_whitespace() {
        let input = "\u{feff}# comment\r\n ; ignored\r\n あ.wav = あ, 1, 2, -3, 4, 5\r\n";
        let parsed = parse_oto_ini(input.as_bytes()).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].alias, "あ");
        assert_eq!(parsed[0].filename, "あ.wav");
        assert_eq!(parsed[0].cutoff, -3.0);
    }

    #[test]
    fn invalid_paths_and_short_rows_are_not_indexed() {
        let input =
            b"../outside.wav=bad,0,0,0,0,0\nC:/outside.wav=bad2,0,0,0,0,0\nshort.wav=short,0,1\n";
        assert!(parse_oto_ini(input).is_err());
    }

    #[test]
    fn blank_alias_uses_wav_stem_and_last_duplicate_wins() {
        let parsed = parse_oto_ini(b"a/KA.WAV=,1,2,3,4,5\nb.wav=KA,6,7,8,9,10\n").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].alias, "KA");
        assert_eq!(parsed[0].offset, 6.0);
    }

    #[test]
    fn indexes_zip_metadata_without_copying_voicebank_samples_to_the_index() {
        let files = vec![
            ("Miku/oto.ini".into(), b"a.wav=a,0,1,2,3,4\n".to_vec()),
            ("Miku/LICENSE.txt".into(), b"CC0".to_vec()),
        ];
        let zip = crate::zip::write(&files).unwrap();
        let index = index_archive(&zip, 1024).unwrap();
        assert_eq!(index.name, "Miku");
        assert_eq!(index.entries.len(), 1);
        assert!(index.has_license_document);
    }
}
