//! What a build reads from `content/`: `sources.json` (the raw source folders, from content-sources'
//! MANIFEST.json) and `recipes/**/*.toml` (one per entry).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use honeydew::content::bundle::Kind;
use serde::Deserialize;

/// One row of `sources.json`: a folder below `--sources`. Rows also carry the manifest's provenance (archives with
/// hashes and commits, retrieval date, attribution, notes); the build reads only what it checks or credits.
#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub folder: String,
    pub title: String,
    pub page: String,
    pub author: String,
    /// SPDX-style id. The manifest's `PD-declared` is `LicenseRef-PD-declared`.
    pub spdx: String,
    #[serde(default)]
    pub archives: Vec<Archive>,
    /// The credit line the credits screen shows. Required for CC-BY.
    pub credit: Option<String>,
    /// The licence notice CREDITS.txt carries in full, relative to the folder. Required for MIT and Iowa MIS.
    pub notice: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Archive {
    pub url: String,
}

/// One entry's recipe. File paths are relative to a source folder.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Stable unique id, `[a-z0-9-]`.
    pub id: String,
    pub kind: Kind,
    /// The entry's place below `Built-in/`.
    pub path: String,
    /// The source folder files come from, unless a pad or sample names its own.
    pub source: String,
    // Program options, for the SFZ and DSP stages. A program reads `sfz`, `sf2`, `files` or `[[zone]]`.
    pub sfz: Option<String>,
    /// A SoundFont patch, rendered at its sample points.
    pub sf2: Option<Sf2>,
    /// A program from a WAV tree without an SFZ: `<folder>/<file name pattern>`, the name with `{note}` (a name such as
    /// `C#4`, or a MIDI number) and optional `{layer}` and `{rr}` (a number) captures.
    pub files: Option<String>,
    /// The `{layer}` values `files` keeps, softest first. Default: every one, numbers in order, then text.
    #[serde(default)]
    pub layers: Vec<String>,
    /// Semitones added to each `{note}`, or to an SFZ region's keys and root: 12 where the names put middle C at C3, as
    /// VSCO 2 and VCSL do, or for an SFZ mapped an octave off its true pitch.
    #[serde(default)]
    pub transpose: i8,
    /// A program's zones, placed as written: the fallback for trees `files` can't describe.
    #[serde(default)]
    pub zone: Vec<Zone>,
    /// Controller values `locc`/`hicc` are evaluated at, by CC number.
    #[serde(default)]
    pub cc: BTreeMap<String, u8>,
    pub keys: Option<Keys>,
    /// Source velocity layers kept, mapped evenly over 1..127.
    #[serde(default)]
    pub velocity_layers: Vec<u8>,
    pub round_robin: Option<u8>,
    /// The articulation kept: regions with another `sw_last` are left out.
    pub keyswitch: Option<u8>,
    /// `auto` (mono when side < −40 dB re mid), `mono` or `stereo`.
    pub channels: Option<String>,
    pub tail: Option<Tail>,
    /// DW attack float (zone floats #1), 0..=1; default 0 for existing programs.
    pub attack: Option<f32>,
    /// DW release float (zone floats #4).
    pub release: Option<f32>,
    pub lite: Option<Lite>,
    /// `normalize`: the loudest sample peaks at −6 dBFS, layers keep their ratios. `even`: the same, after each key's
    /// layers are scaled alike so its loudest layer's RMS matches every other key's.
    pub gain: Option<String>,
    #[serde(rename = "loop")]
    pub looping: Option<Loop>,
    /// Kit pads, one sample each.
    #[serde(default)]
    pub pad: Vec<Pad>,
    /// A `samples` folder's files.
    #[serde(default)]
    pub sample: Vec<Sample>,
    /// A procedural model (`crate::procedural`) rendering a `samples` folder: one member per `[[variant]]`.
    pub render: Option<String>,
    #[serde(default)]
    pub variant: Vec<Variant>,
}

/// One render of a `samples` recipe's model: `<name>.flac`, with the model's parameters beside the name.
#[derive(Debug, Clone, Deserialize)]
pub struct Variant {
    pub name: String,
    #[serde(flatten)]
    pub params: toml::Table,
}

/// One `[[zone]]`: `file` plays at its own pitch, `tune` cents up (default 0), on `root`, over `lokey..=hikey` (default
/// the root alone) and `lovel..=hivel` (default 0–127).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub file: String,
    pub root: u8,
    pub lokey: Option<u8>,
    pub hikey: Option<u8>,
    pub lovel: Option<u8>,
    pub hivel: Option<u8>,
    #[serde(default)]
    pub tune: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sf2 {
    pub file: String,
    /// The bank-0 (GM) program.
    pub preset: u8,
    /// The keys rendered: the sample points, best the patch's own roots.
    pub keys: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    pub every: u8,
    pub from: Option<u8>,
    pub to: Option<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tail {
    pub floor_db: f32,
    pub fade_ms: f32,
    pub max_s: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lite {
    pub every: u8,
    pub layer: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loop {
    pub find: bool,
    pub min_s: f32,
    pub crossfade_ms: f32,
    pub keep_s: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pad {
    pub name: String,
    pub level_db: Option<f32>,
    pub pan: Option<f32>,
    pub source: Option<String>,
    /// Time-aligned mic takes summed into one stereo pair.
    #[serde(default)]
    pub mix: Vec<Mix>,
    /// A procedural model rendering the pad instead of `mix`, with its `params`.
    pub render: Option<String>,
    #[serde(default)]
    pub params: toml::Table,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    pub file: String,
    #[serde(default)]
    pub gain_db: f32,
    pub pan: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    /// Display name: the member is `<name>.flac`.
    pub name: String,
    pub file: String,
    pub source: Option<String>,
}

impl Recipe {
    /// Every input file the recipe names, as (source folder, path in it).
    pub fn files(&self) -> Vec<(&str, &str)> {
        let own = (self.sfz.iter().chain(self.sf2.iter().map(|s| &s.file)))
            .chain(&self.files)
            .chain(self.zone.iter().map(|z| &z.file));
        let mut out: Vec<(&str, &str)> = own.map(|f| (self.source.as_str(), f.as_str())).collect();
        for p in &self.pad {
            let src = p.source.as_deref().unwrap_or(&self.source);
            out.extend(p.mix.iter().map(|m| (src, m.file.as_str())));
        }
        out.extend(
            self.sample
                .iter()
                .map(|s| (s.source.as_deref().unwrap_or(&self.source), s.file.as_str())),
        );
        out
    }

    /// Every source folder the recipe reads: its credits.
    pub fn sources(&self) -> BTreeSet<&str> {
        let mut out: BTreeSet<&str> = self.files().into_iter().map(|(s, _)| s).collect();
        out.insert(&self.source);
        out
    }

    /// Names the pickers show: the path's folders and stem, pads and samples.
    pub fn display_names(&self) -> impl Iterator<Item = &str> {
        self.path
            .split('/')
            .chain(self.pad.iter().map(|p| p.name.as_str()))
            .chain(self.sample.iter().map(|s| s.name.as_str()))
            .chain(self.variant.iter().map(|v| v.name.as_str()))
    }
}

pub fn sources(content: &Path) -> Result<BTreeMap<String, Source>, String> {
    let path = content.join("sources.json");
    let rows: Vec<Source> =
        serde_json::from_slice(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(rows.into_iter().map(|s| (s.folder.clone(), s)).collect())
}

/// Every `recipes/**/*.toml`, in path order.
pub fn recipes(content: &Path) -> Result<Vec<Recipe>, String> {
    let mut files = Vec::new();
    walk(&content.join("recipes"), &mut files)?;
    files.sort();
    files
        .iter()
        .map(|f| {
            let text = String::from_utf8(read(f)?).map_err(|e| format!("{}: {e}", f.display()))?;
            toml::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))
        })
        .collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "toml") {
            out.push(path);
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}
