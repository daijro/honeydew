//! Voicebank metadata and UTAU `oto.ini` support for Honeydew's Vocal Synth.
//!
//! Voicebank files are user content: this module only indexes metadata and never bundles or republishes samples. A
//! bank's own license must be checked before sharing a project that depends on it.

use std::collections::BTreeMap;

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
    pub entries: Vec<OtoEntry>,
    /// A license/readme file was found; its contents are deliberately not interpreted as a license grant.
    pub has_license_document: bool,
}

/// Parse a UTAU `oto.ini` into a stable alias-sorted list.
///
/// The alias parsing follows the MIT-licensed WebTAU UTAU metadata parser, adapted for Honeydew's safe relative-path
/// checks and Shift-JIS decoding. See `THIRD_PARTY_NOTICES.md` and `licenses/webtau-MIT.txt`.
pub fn parse_oto_ini(data: &[u8]) -> Result<Vec<OtoEntry>, String> {
    let decoded = match std::str::from_utf8(data) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => encoding_rs::SHIFT_JIS.decode(data).0,
    };
    let mut entries = BTreeMap::<String, OtoEntry>::new();

    for line in decoded.lines() {
        let line = line.trim().trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((filename, values)) = line.split_once('=') else {
            continue;
        };
        let filename = filename.trim().replace('\\', "/");
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
            raw_alias.to_owned()
        };
        let values: Vec<f64> = values
            .take(5)
            .map(|value| value.trim().parse::<f64>().unwrap_or(0.0))
            .collect();
        if values.len() < 5 || alias.is_empty() {
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

/// Parse the first `oto.ini` in a `.zip`/`.uar` voicebank using Honeydew's bounded ZIP reader.
///
/// This intentionally indexes metadata only; samples are not extracted or embedded in the project.
pub fn index_archive(bytes: &[u8], max_unpacked: u64) -> Result<VoicebankIndex, String> {
    let files = crate::zip::read(bytes, max_unpacked).map_err(|err| err.to_string())?;
    let oto = files
        .iter()
        .filter(|entry| {
            entry
                .path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("oto.ini"))
        })
        .min_by_key(|entry| entry.path.matches('/').count())
        .ok_or_else(|| "This archive does not contain an oto.ini voicebank index".to_owned())?;
    let entries = parse_oto_ini(&oto.data)?;
    let root = oto.path.rsplit_once('/').map_or("", |(root, _)| root);
    let name = root
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Voicebank")
        .to_owned();
    let has_license_document = files.iter().any(|entry| {
        let path = entry.path.to_ascii_lowercase();
        let basename = path.rsplit('/').next().unwrap_or(&path);
        basename.starts_with("license") || basename.starts_with("licence")
    });
    Ok(VoicebankIndex {
        name,
        oto_path: oto.path.clone(),
        entries,
        has_license_document,
    })
}

fn safe_sample_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let input = b"../outside.wav=bad,0,0,0,0,0\nC:/outside.wav=bad2,0,0,0,0,0\nshort.wav=short,0,1\n";
        assert!(parse_oto_ini(input).is_err());
    }

    #[test]
    fn blank_alias_uses_wav_stem_and_last_duplicate_wins() {
        let parsed = parse_oto_ini(b"a/KA.WAV=,1,2,3,4,5\nb.wav=ka,6,7,8,9,10\n").unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].alias, "ka");
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
