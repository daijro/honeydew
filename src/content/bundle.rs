//! Bundled content: the packs `tools/content-build` writes and the app reads below the
//! factory root, as `@R@Built-in/<path>`. [`Index`] is `content/index.json`, schema 1: every entry, its pack parts and
//! where each member's bytes sit in them. The build tool serialises it; the app parses it hostile-safe.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project_files::MAX_FILE_BYTES;

/// The only index schema this build reads.
pub const SCHEMA: u32 = 1;

/// `content/index.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub schema: u32,
    pub release: String,
    /// Old path → new path, both below `Built-in/`: a release never drops a path songs may store.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// Sources by key, for the credits screen.
    #[serde(default)]
    pub credits: BTreeMap<String, Credit>,
    pub entries: Vec<IndexEntry>,
}

/// One credited source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credit {
    pub title: String,
    pub author: String,
    /// SPDX-style id.
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an entry is, and so which picker lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A DW Sampler program: `Presets/DW Sampler/<Category>/<Name>.dwp`.
    Program,
    /// A drumset: `Drumsets/<Name>.tmpl`.
    Kit,
    /// A folder of samples: `Samples/<Category>/`, `Loops/` or `Shapes/` (trailing `/`).
    Samples,
}

/// One entry: an instrument, a kit or a sample folder, fetched and evicted as a unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Stable unique id, `[a-z0-9-]`.
    pub id: String,
    pub kind: Kind,
    /// Its place below `Built-in/`. The picker folder is its parent, the display name its stem.
    pub path: String,
    /// SPDX-style id.
    pub license: String,
    /// Keys into [`Index::credits`].
    #[serde(default)]
    pub credits: Vec<String>,
    /// f32 bytes once fully decoded.
    #[serde(default)]
    pub decoded: u64,
    /// The lite program's member name, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lite: Option<String>,
    pub parts: Vec<Part>,
    pub members: Vec<Member>,
}

/// One pack part: an uncompressed ustar file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Relative to `content/`: `p/<hash16>.tar`.
    pub file: String,
    /// Raw size.
    pub bytes: u64,
    /// Base64 of the raw part's SHA-256.
    pub sha256: String,
}

/// One member, serialised as `[name, part, offset, length]`: its name relative to the entry's base (the folder holding
/// `path` for a program or kit, `path` itself for a folder), the index of the part holding it, and its data's offset
/// and length in that part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member(pub String, pub usize, pub u64, pub u64);

/// The largest `index.json` read: about 160 KB is expected.
pub const MAX_INDEX_BYTES: usize = 1 << 20;

impl Index {
    /// The index in `bytes`, or `None` (no bundle) when it is over [`MAX_INDEX_BYTES`], is no schema-1 index, or names
    /// a path that could leave its folder: empty, absolute, with a `\`, `:` or NUL, or with an empty, `.` or `..` part.
    /// A member must sit in one of its entry's parts, and a part's file be named `p/<16 hex digits>.tar`.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_INDEX_BYTES {
            return None;
        }
        let index: Self = serde_json::from_slice(bytes).ok()?;
        let entries_ok = index.entries.iter().all(|e| {
            let path = match e.kind {
                Kind::Samples => e.path.strip_suffix('/'),
                Kind::Program | Kind::Kit => Some(e.path.as_str()),
            };
            path.is_some_and(safe)
                && e.lite.as_deref().is_none_or(safe)
                && e.parts.iter().all(|p| part_name(&p.file))
                && (e.members.iter()).all(|m| safe(&m.0) && m.1 < e.parts.len())
        });
        let aliases_ok = (index.aliases.iter()).all(|(old, new)| safe(old) && safe(new));
        (index.schema == SCHEMA && entries_ok && aliases_ok).then_some(index)
    }

    /// A program's displayed category, base name and credited source. Only a matching source suffix is stripped.
    fn program_name<'a>(&self, entry: &'a IndexEntry) -> Option<(&'a str, &'a str, String)> {
        if entry.kind != Kind::Program {
            return None;
        }
        let (folder, file) = entry
            .path
            .strip_prefix("Presets/DW Sampler/")?
            .rsplit_once('/')?;
        let category = crate::ui::preset_picker::categories::category(1, folder)?;
        let stem = Path::new(file).file_stem()?.to_str()?;
        let sources: Option<Vec<_>> = entry
            .credits
            .iter()
            .map(|id| match id.as_str() {
                "vsco2-ce" => Some("VSCO 2 CE"),
                "vcsl" => Some("VCSL"),
                "iowa-strings" => Some("Iowa"),
                "meta-nylon-guitar" | "meta-steel-guitar" => Some("Meta"),
                _ => self.credits.get(id).map(|credit| credit.title.as_str()),
            })
            .collect();
        let source = sources?.join(", ");
        if source.is_empty() {
            return None;
        }
        let suffix = format!(" ({source})");
        Some((category, stem.strip_suffix(&suffix).unwrap_or(stem), source))
    }
}

/// `name` is a plain relative `/`-separated path that stays in its folder.
fn safe(name: &str) -> bool {
    !name.contains(['\\', ':', '\0'])
        && (name.split('/')).all(|part| !matches!(part, "" | "." | ".."))
}

/// `file` is a part's content-hashed name: `p/` and 16 lowercase hex digits, then `.tar`.
fn part_name(file: &str) -> bool {
    let hash = file.strip_prefix("p/").and_then(|f| f.strip_suffix(".tar"));
    hash.is_some_and(|h| h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// The file at `path`, opened when it is a regular one: a FIFO or a device would block or never end.
pub(super) fn regular(path: &Path) -> Option<std::fs::File> {
    let file = std::fs::metadata(path).ok()?.is_file();
    file.then(|| std::fs::File::open(path).ok())?
}

/// An entry's base: member names are relative to it (`""` at the top).
fn base(entry: &IndexEntry) -> &str {
    match entry.kind {
        Kind::Samples => entry.path.trim_end_matches('/'),
        Kind::Program | Kind::Kit => entry.path.rsplit_once('/').map_or("", |(dir, _)| dir),
    }
}

/// The same bounded member read for disk packs and Android APK assets.
fn read_member(mut reader: impl Read + Seek, offset: u64, len: u64) -> Option<Vec<u8>> {
    if len > MAX_FILE_BYTES {
        return None;
    }
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    reader.take(len).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 == len).then_some(bytes)
}

/// The bundled content at a content dir, or on the web in the parts the loader downloaded ([`super::web`]): its index,
/// and where each member is.
pub struct Bundle {
    pub index: Index,
    /// The content dir; `None` on the web.
    pub(super) dir: Option<PathBuf>,
    /// Each member's path below `Built-in/` → its entry and member.
    members: HashMap<String, (usize, usize)>,
}

/// The web's bundle, its index downloaded at start ([`install`]), until [`Bundle::find`] takes it.
static INSTALLED: std::sync::Mutex<Option<Bundle>> = std::sync::Mutex::new(None);

/// The web's bundle, of `index`: before the first [`super::factory`] call, which takes it.
pub fn install(index: Index) {
    if let Ok(mut installed) = INSTALLED.lock() {
        *installed = Some(Bundle::new(index, None));
    }
}

impl Bundle {
    /// The bundle in `dir` (`dir/index.json`, [`Index::parse`]), `None` without a readable index.
    pub fn open(dir: &Path) -> Option<Self> {
        let mut bytes = Vec::new();
        let file = regular(&dir.join("index.json"))?;
        file.take(MAX_INDEX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(Self::new(Index::parse(&bytes)?, Some(dir.to_owned())))
    }

    /// The bundle of `index`, its parts in `dir`, or on the web (`None`) in the loader's.
    pub fn new(index: Index, dir: Option<PathBuf>) -> Self {
        let mut members = HashMap::new();
        for (e, entry) in index.entries.iter().enumerate() {
            for (m, Member(name, ..)) in entry.members.iter().enumerate() {
                let path = match base(entry) {
                    "" => name.clone(),
                    base => format!("{base}/{name}"),
                };
                members.entry(path).or_insert((e, m));
            }
        }
        Self {
            index,
            dir,
            members,
        }
    }

    /// The desktop's bundle ([`Self::find_in`]) of `$HONEYDEW_CONTENT_DIR`, the installed library
    /// ([`super::install::dir`]) and the one beside the app: `<exe dir>/content` or `<exe dir>/../share/honeydew/content`
    /// (macOS `../Resources/content`), which then syncs into the installed one on a worker. Never in unit tests, which
    /// build their own. On the web and Android, the one [`install`]ed.
    pub fn find() -> Option<Self> {
        #[cfg(any(target_arch = "wasm32", target_os = "android"))]
        return INSTALLED.lock().ok()?.take();
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        {
            if cfg!(test) {
                return None;
            }
            let exe = std::env::current_exe().ok();
            let beside = exe.as_deref().and_then(Path::parent).map(|dir| {
                let installed = match cfg!(target_os = "macos") {
                    true => "../Resources/content",
                    false => "../share/honeydew/content",
                };
                [dir.join("content"), dir.join(installed)]
            });
            let var = std::env::var_os("HONEYDEW_CONTENT_DIR").map(PathBuf::from);
            let installed = super::install::dir();
            let beside: Vec<_> = beside.into_iter().flatten().collect();
            let (bundle, sync) = Self::find_in(var.as_deref(), installed.as_deref(), &beside)?;
            if let (Some(from), Some(to)) = (sync, installed) {
                super::install::sync_in_background(from, to);
            }
            Some(bundle)
        }
    }

    /// [`Self::find`] in `var`, the installed library and the folders `beside` the app: `var`'s; the installed one
    /// while it is complete and, with a library beside the app, of that one's index; the first beside the app, and its
    /// folder to sync the installed one from, unless its release is older than the installed one's; or the installed
    /// one as it is (a sync cut short leaves one that works).
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    fn find_in(
        var: Option<&Path>,
        installed: Option<&Path>,
        beside: &[PathBuf],
    ) -> Option<(Self, Option<PathBuf>)> {
        use super::install::{complete, older, same_index};
        if let Some(bundle) = var.and_then(Self::open) {
            return Some((bundle, None));
        }
        let shipped = beside.iter().find_map(|dir| Some((Self::open(dir)?, dir)));
        let current = installed.filter(|dir| complete(dir));
        let in_place = installed.and_then(Self::open);
        // Never back to an older release than the installed one.
        let downgrade = |bundle: &Self| {
            in_place
                .as_ref()
                .is_some_and(|i| older(&bundle.index.release, &i.index.release))
        };
        match shipped {
            Some((bundle, dir))
                if !current.is_some_and(|current| same_index(current, dir))
                    && !downgrade(&bundle) =>
            {
                Some((bundle, installed.map(|_| dir.clone())))
            }
            shipped => in_place
                .or(shipped.map(|(bundle, _)| bundle))
                .map(|bundle| (bundle, None)),
        }
    }

    /// `rel`, below `Built-in/` (aliases applied): its entry and its member.
    fn member(&self, rel: &str) -> Option<(&IndexEntry, &Member)> {
        let rel = self.index.aliases.get(rel).map_or(rel, String::as_str);
        let &(e, m) = self.members.get(rel)?;
        let entry = &self.index.entries[e];
        Some((entry, &entry.members[m]))
    }

    /// The entry that holds `rel`, below `Built-in/` (aliases applied).
    pub fn entry_of(&self, rel: &str) -> Option<&IndexEntry> {
        self.member(rel).map(|(entry, _)| entry)
    }

    /// A colliding program's display filename, including its source; its stored path and pack members never change.
    pub(super) fn display_name(&self, rel: &str) -> Option<String> {
        let entry = self.entry_of(rel)?;
        if entry.path != rel {
            return None;
        }
        let (category, name, source) = self.index.program_name(entry)?;
        let duplicate = self
            .index
            .entries
            .iter()
            .filter(|e| e.path != entry.path)
            .any(|e| {
                self.index
                    .program_name(e)
                    .is_some_and(|(other_category, other_name, _)| {
                        category == other_category && name == other_name
                    })
            });
        let ext = Path::new(rel).extension()?.to_str()?;
        duplicate.then(|| format!("{name} ({source}).{ext}"))
    }

    /// The bytes of the member at `rel`, below `Built-in/` (aliases applied), read from its part. `None` when no member
    /// is there, its part is missing or short, or it is over [`MAX_FILE_BYTES`]. On the web a program with a lite one
    /// reads as that until all its parts are in: its zones are the same files.
    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let (entry, member) = self.member(rel)?;
        if member.3 > MAX_FILE_BYTES {
            return None;
        }
        #[cfg(target_os = "android")]
        if self.dir.is_none() {
            let Member(_, part, offset, len) = member;
            let asset = crate::android::asset(&format!("content/{}", entry.parts[*part].file))?;
            return read_member(asset, *offset, *len);
        }
        let Some(dir) = &self.dir else {
            let program = Some(member.0.as_str()) == entry.path.rsplit('/').next();
            let lite = (entry.lite.as_ref()).filter(|_| program && !super::web::all_in(entry));
            let lite = lite.and_then(|name| entry.members.iter().find(|m| m.0 == *name));
            let Member(_, part, offset, len) = lite.unwrap_or(member);
            return super::web::slice(&entry.parts[*part].file, *offset, *len);
        };
        let Member(_, part, offset, len) = member;
        read_member(regular(&dir.join(&entry.parts[*part].file))?, *offset, *len)
    }

    /// `CREDITS.txt` beside the index: the full licence notices the credits screen shows. `None` without a readable
    /// one.
    pub fn notices(&self) -> Option<String> {
        #[cfg(target_os = "android")]
        if self.dir.is_none() {
            return String::from_utf8(crate::android::asset_bytes(
                "content/CREDITS.txt",
                MAX_INDEX_BYTES,
            )?)
            .ok();
        }
        let mut text = String::new();
        let file = regular(&self.dir.as_ref()?.join("CREDITS.txt"))?;
        (file.take(MAX_INDEX_BYTES as u64).read_to_string(&mut text)).ok()?;
        Some(text)
    }

    /// The members a picker lists, below `Built-in/`: each program's and kit's file, and every member of a folder.
    /// Zone files and lite programs are read only through a program.
    pub fn listed(&self) -> Vec<String> {
        let mut listed = Vec::new();
        for entry in &self.index.entries {
            match entry.kind {
                Kind::Program | Kind::Kit => listed.push(entry.path.clone()),
                Kind::Samples => listed.extend(
                    (entry.members.iter()).map(|Member(name, ..)| format!("{}{name}", entry.path)),
                ),
            }
        }
        listed
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fixture entry: its kind, its path and its members with their bytes.
    pub(crate) type Fixture<'a> = (Kind, &'a str, Vec<(&'a str, Vec<u8>)>);

    /// A bundle in `dir` of `entries`, each in a part of its own, opened.
    pub(crate) fn fixture(dir: &Path, entries: &[Fixture<'_>]) -> Bundle {
        std::fs::create_dir_all(dir.join("p")).unwrap();
        let entries = entries
            .iter()
            .enumerate()
            .map(|(n, (kind, path, members))| {
                let (file, mut part) = (format!("p/{n:016x}.tar"), Vec::new());
                let members = (members.iter())
                    .map(|(name, bytes)| {
                        let at = part.len() as u64;
                        part.extend_from_slice(bytes);
                        Member(name.to_string(), 0, at, bytes.len() as u64)
                    })
                    .collect();
                std::fs::write(dir.join(&file), &part).unwrap();
                let bytes = part.len() as u64;
                IndexEntry {
                    id: format!("e{n}"),
                    kind: *kind,
                    path: path.to_string(),
                    license: "CC0-1.0".into(),
                    credits: Vec::new(),
                    decoded: 0,
                    lite: None,
                    parts: vec![Part {
                        file,
                        bytes,
                        sha256: String::new(),
                    }],
                    members,
                }
            });
        let index = Index {
            schema: SCHEMA,
            release: "test".into(),
            aliases: BTreeMap::new(),
            credits: BTreeMap::new(),
            entries: entries.collect(),
        };
        std::fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
        Bundle::open(dir).unwrap()
    }

    #[test]
    fn the_index_is_schema_1_under_the_cap_and_names_no_path_that_leaves_its_folder() {
        let good = r#"{"schema": 1, "release": "r", "aliases": {"Samples/Kick/Old.flac": "Samples/Kick/K1.flac"},
            "entries": [{"id": "kick", "kind": "samples", "path": "Samples/Kick/", "license": "CC0-1.0",
            "parts": [{"file": "p/0123456789abcdef.tar", "bytes": 4, "sha256": ""}], "members": [["K1.flac", 0, 0, 4]]}]}"#;
        assert!(Index::parse(good.as_bytes()).is_some());
        for (from, to) in [
            (r#""schema": 1"#, r#""schema": 2"#),
            (r#""K1.flac", 0"#, r#""../K1.flac", 0"#),
            (r#""K1.flac", 0"#, r#""/K1.flac", 0"#),
            (r#""K1.flac", 0"#, r#""", 0"#),
            (r#""K1.flac", 0"#, r#""K1.flac", 1"#),
            ("p/0123456789abcdef.tar", "C:/p/0123456789abcdef.tar"),
            ("p/0123456789abcdef.tar", "p/0123456789abcdeX.tar"),
            ("Kick/K1.flac", "Kick/../K1.flac"),
            ("Samples/Kick/\"", "Samples/Kick\""),
        ] {
            let bad = good.replace(from, to);
            assert!(Index::parse(bad.as_bytes()).is_none(), "{to}");
        }
        let big = format!("{good}{}", " ".repeat(MAX_INDEX_BYTES));
        assert!(Index::parse(big.as_bytes()).is_none());
    }

    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    #[test]
    fn the_installed_library_serves_while_complete_and_one_beside_the_app_syncs_into_it() {
        use crate::content::install::{MANIFEST, Progress, sync};
        let tmp = crate::project_files::content::tests::Temp::new("bundle-find");
        let [var, installed, beside] = ["var", "installed", "beside"].map(|d| tmp.0.join(d));
        let library = |dir: &Path, bytes: &[u8]| {
            fixture(
                dir,
                &[(Kind::Samples, "Loops/", vec![("A.flac", bytes.to_vec())])],
            );
        };
        // The folder used and the one to sync from.
        let find = |var: Option<&Path>| {
            let found = Bundle::find_in(var, Some(&installed), std::slice::from_ref(&beside));
            found.map(|(bundle, from)| (bundle.dir.unwrap(), from))
        };
        assert!(find(None).is_none());
        library(&beside, b"one");
        assert_eq!(find(None), Some((beside.clone(), Some(beside.clone()))));
        sync(&beside, &installed, &Progress::default()).unwrap();
        assert_eq!(find(None), Some((installed.clone(), None)));
        // An app-only download: nothing beside it.
        std::fs::remove_dir_all(&beside).unwrap();
        assert_eq!(find(None), Some((installed.clone(), None)));
        // A sync cut short before its manifest: still the installed one, until a library beside the app syncs again.
        std::fs::remove_file(installed.join(MANIFEST)).unwrap();
        assert_eq!(find(None), Some((installed.clone(), None)));
        library(&beside, b"one");
        assert_eq!(find(None), Some((beside.clone(), Some(beside.clone()))));
        // A new version's library beside the app.
        let release = |dir: &Path, release: &str| {
            let mut index = Bundle::open(dir).unwrap().index;
            index.release = release.into();
            std::fs::write(dir.join("index.json"), serde_json::to_vec(&index).unwrap()).unwrap();
        };
        sync(&beside, &installed, &Progress::default()).unwrap();
        library(&beside, b"two!");
        release(&beside, "0.2.0");
        assert_eq!(find(None), Some((beside.clone(), Some(beside.clone()))));
        // Never back: an older release beside the app leaves the installed one as it is.
        sync(&beside, &installed, &Progress::default()).unwrap();
        library(&beside, b"one");
        release(&beside, "0.1.0");
        assert_eq!(find(None), Some((installed.clone(), None)));
        library(&var, b"var");
        assert_eq!(find(Some(&var)), Some((var.clone(), None)));
    }
}

#[cfg(test)]
mod member_read_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn disk_and_apk_members_are_bounded_and_never_return_partial_samples() {
        let data = || Cursor::new(b"headerSAMPLEpadding");
        assert_eq!(read_member(data(), 6, 6).as_deref(), Some(&b"SAMPLE"[..]));
        assert!(read_member(data(), 6, 40).is_none());
        assert!(read_member(data(), 100, 1).is_none());
        assert!(read_member(data(), 0, MAX_FILE_BYTES + 1).is_none());
    }
}
