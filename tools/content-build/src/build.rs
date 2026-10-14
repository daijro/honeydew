//! `content-build build`: gates, each recipe's members, packs, then `index.json`, `CREDITS.txt` and their
//! `.gz` siblings, then [`crate::verify`] over the result.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use honeydew::content::bundle::{Credit, Index, IndexEntry, Kind, Member, Part, SCHEMA};

use crate::pack::{self, CAP};
use crate::recipe::{self, Recipe, Source};
use crate::sfz::{self, Region};

pub struct Opts {
    /// `content/`: sources.json, recipes/, inputs.lock.
    pub content: PathBuf,
    /// The raw source folders (content-sources).
    pub sources: PathBuf,
    pub out: PathBuf,
    /// Build only these recipe ids (the index then lists only them).
    pub only: Vec<String>,
    /// Record new or changed input hashes instead of failing on them.
    pub update_lock: bool,
    pub release: String,
}

/// Every source file a build opens, checked against `content/inputs.lock` (`<sha256 hex>  <folder>/<path>` lines):
/// the upstream archives are gone, so per-file hashes are what pin the tree.
pub struct Inputs {
    root: PathBuf,
    lock: BTreeMap<String, String>,
    update: bool,
    /// The lock keys this build read.
    opened: BTreeSet<String>,
}

impl Inputs {
    pub fn new(root: &Path, lock_text: &str, update: bool) -> Self {
        let lock = lock_text
            .lines()
            .filter_map(|l| l.split_once("  "))
            .map(|(h, k)| (k.into(), h.into()))
            .collect();
        Self {
            root: root.into(),
            lock,
            update,
            opened: BTreeSet::new(),
        }
    }

    /// An audio or SFZ input (whitelisted extension), hash-checked.
    pub fn read(&mut self, source: &str, rel: &str) -> Result<Vec<u8>, String> {
        crate::gates::input(rel)?;
        self.read_locked(source, rel)
    }

    /// The regions of the SFZ input `rel`, controllers at `cc` ([`sfz::parse`]). It and its `#include`s, which may be
    /// `.txt`, are read hash-checked.
    pub fn sfz(
        &mut self,
        source: &str,
        rel: &str,
        cc: &BTreeMap<u8, u8>,
    ) -> Result<Vec<Region>, String> {
        crate::gates::input(rel)?;
        let mut read = |path: &Path| {
            let rel = path.to_string_lossy();
            crate::gates::sfz_text(&rel)
                .and_then(|()| self.read_locked(source, &rel))
                .map(|text| String::from_utf8_lossy(&text).into_owned())
                .map_err(std::io::Error::other)
        };
        sfz::parse(Path::new(rel), cc, &mut read).map_err(|e| format!("{source}/{rel}: {e}"))
    }

    /// The file names in the folder `rel` of `source`, sorted. Each is still opened through [`Self::read`].
    // ponytail: the listing itself isn't locked, so a removed file shrinks a program silently; a new one still fails
    // the lock. Hash the listing into the lock if the trees ever change upstream.
    pub fn list(&self, source: &str, rel: &str) -> Result<Vec<String>, String> {
        let dir = self.inside(source, rel)?;
        let mut names = Vec::new();
        for e in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            if e.file_type().is_ok_and(|t| !t.is_dir()) {
                names.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// `rel` in `source`, symlinks resolved; an error unless it sits inside the source folder.
    fn inside(&self, source: &str, rel: &str) -> Result<PathBuf, String> {
        let dir = self.root.join(source);
        let real = |p: &Path| {
            p.canonicalize()
                .map_err(|e| format!("{}: {e}", p.display()))
        };
        let path = real(&dir.join(rel))?;
        if !path.starts_with(real(&dir)?) {
            return Err(format!(
                "{source}/{rel}: resolves outside its source folder"
            ));
        }
        Ok(path)
    }

    /// Every read (`read`, `sfz`, notices) lands here: the file, symlinks resolved, must sit inside its source folder.
    fn read_locked(&mut self, source: &str, rel: &str) -> Result<Vec<u8>, String> {
        let key = format!("{source}/{rel}");
        let bytes = recipe::read(&self.inside(source, rel)?)?;
        let hash = pack::hex(&pack::sha256(&bytes));
        self.opened.insert(key.clone());
        match self.lock.get(&key) {
            Some(h) if *h == hash => {}
            _ if self.update => {
                self.lock.insert(key, hash);
            }
            Some(_) => {
                return Err(format!(
                    "{key}: changed since inputs.lock (pass --update-lock to accept)"
                ));
            }
            None => {
                return Err(format!(
                    "{key}: not in inputs.lock (pass --update-lock to add it)"
                ));
            }
        }
        Ok(bytes)
    }

    /// The lock as `inputs.lock` holds it; with `prune`, only the files this build read.
    pub fn lock_text(&self, prune: bool) -> String {
        (self.lock.iter())
            .filter(|(k, _)| !prune || self.opened.contains(*k))
            .map(|(k, h)| format!("{h}  {k}\n"))
            .collect()
    }
}

pub fn build(o: &Opts) -> Result<(), String> {
    let sources = recipe::sources(&o.content)?;
    let recipes = recipe::recipes(&o.content)?;
    crate::gates::check(&recipes, &sources)?;
    let lock_path = o.content.join("inputs.lock");
    let lock = fs::read_to_string(&lock_path).unwrap_or_default();
    let mut inputs = Inputs::new(&o.sources, &lock, o.update_lock);

    let p = o.out.join("p");
    fs::create_dir_all(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    for e in fs::read_dir(&p).map_err(|e| e.to_string())? {
        let path = e.map_err(|e| e.to_string())?.path();
        if path
            .to_str()
            .is_some_and(|s| s.ends_with(".tar") || s.ends_with(".tar.gz"))
        {
            fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }

    let mut entries = Vec::new();
    let mut credits = BTreeMap::new();
    for r in recipes
        .iter()
        .filter(|r| o.only.is_empty() || o.only.contains(&r.id))
    {
        let members = members(r, &mut inputs)?;
        entries.push(entry(r, &sources, &members, &o.out)?);
        for s in r.sources() {
            credits.insert(s.to_owned(), credit(&sources[s]));
        }
    }
    let index = Index {
        schema: SCHEMA,
        release: o.release.clone(),
        aliases: BTreeMap::new(),
        credits,
        entries,
    };
    let json = serde_json::to_vec(&index).map_err(|e| e.to_string())?;
    with_gz(&o.out.join("index.json"), &json, &pack::gzip(&json))?;
    let credits = credits_txt(&index, &sources, &mut inputs)?;
    with_gz(
        &o.out.join("CREDITS.txt"),
        credits.as_bytes(),
        &pack::gzip(credits.as_bytes()),
    )?;
    write(
        &o.out.join("sizes.txt"),
        sizes_txt(&index, &o.out)?.as_bytes(),
    )?;
    if o.update_lock {
        // A partial build (`--only`) keeps the other recipes' lines.
        write(&lock_path, inputs.lock_text(o.only.is_empty()).as_bytes())?;
    }
    crate::verify::verify(&o.out, None)
}

/// `sizes.txt`: each entry's parts raw and gzipped and its decoded f32 bytes, then the same per
/// category (the folder holding the entry's path) and in all.
fn sizes_txt(index: &Index, out: &Path) -> Result<String, String> {
    let mb = |b: u64| b as f64 / 1e6;
    let row = |[raw, gz, dec]: [u64; 3], name: &str| {
        format!(
            "{:>9.2} {:>9.2} {:>9.2}  {name}\n",
            mb(raw),
            mb(gz),
            mb(dec)
        )
    };
    let mut t = String::from("  raw MB     gz MB  f32 MB\n");
    let mut categories: BTreeMap<&str, [u64; 3]> = BTreeMap::new();
    let mut all = [0; 3];
    for e in &index.entries {
        let mut sizes = [0, 0, e.decoded];
        for p in &e.parts {
            let gz = out.join(format!("{}.gz", p.file));
            sizes[0] += p.bytes;
            sizes[1] += fs::metadata(&gz)
                .map_err(|e| format!("{}: {e}", gz.display()))?
                .len();
        }
        t += &row(sizes, &format!("{} ({})", e.id, e.path));
        let category = e
            .path
            .trim_end_matches('/')
            .rsplit_once('/')
            .map_or("", |(c, _)| c);
        let sum = categories
            .entry(if e.kind == Kind::Samples {
                e.path.trim_end_matches('/')
            } else {
                category
            })
            .or_default();
        for i in 0..3 {
            sum[i] += sizes[i];
            all[i] += sizes[i];
        }
    }
    t += "\n";
    for (c, sizes) in categories {
        t += &row(sizes, c);
    }
    Ok(t + &row(all, "all"))
}

/// Member names relative to the entry base, and their bytes.
pub(crate) type Members = Vec<(String, Vec<u8>)>;

/// The entry's members in priority order, and how many lead the lite set.
fn members(r: &Recipe, inputs: &mut Inputs) -> Result<(Members, usize), String> {
    if let Some(rendered) = crate::procedural::members(r)? {
        return Ok(rendered);
    }
    match r.kind {
        Kind::Samples => crate::stages::samples(r, inputs),
        Kind::Program => crate::stages::program(r, inputs),
        Kind::Kit => crate::stages::kit(r, inputs),
    }
}

/// Packs one entry's members into `out/p/` and returns its index row.
fn entry(
    r: &Recipe,
    sources: &BTreeMap<String, Source>,
    (members, lite): &(Members, usize),
    out: &Path,
) -> Result<IndexEntry, String> {
    let mut decoded = 0;
    for (name, data) in members.iter().filter(|(n, _)| n.ends_with(".flac")) {
        let d =
            honeydew_audio::sample::decode(data).map_err(|e| format!("{}: {name}: {e:?}", r.id))?;
        decoded += (d.left.len() * (1 + usize::from(d.right.is_some())) * 4) as u64;
    }
    let mut parts = Vec::new();
    let mut rows = vec![None; members.len()];
    for (i, part) in pack::split(members, *lite, CAP)
        .map_err(|e| format!("{}: {e}", r.id))?
        .into_iter()
        .enumerate()
    {
        let sha = pack::sha256(&part.tar);
        let file = format!("p/{}.tar", &pack::hex(&sha)[..16]);
        with_gz(&out.join(&file), &part.tar, &part.gz)?;
        for (m, offset) in part.members {
            rows[m] = Some(Member(
                members[m].0.clone(),
                i,
                offset,
                members[m].1.len() as u64,
            ));
        }
        parts.push(Part {
            file,
            bytes: part.tar.len() as u64,
            sha256: pack::base64(&sha),
        });
    }
    Ok(IndexEntry {
        id: r.id.clone(),
        kind: r.kind,
        path: r.path.clone(),
        license: sources[&r.source].spdx.clone(),
        credits: r.sources().into_iter().map(str::to_owned).collect(),
        decoded,
        lite: members
            .iter()
            .find(|(n, _)| n.ends_with("/lite.dwp"))
            .map(|(n, _)| n.clone()),
        parts,
        members: rows
            .into_iter()
            .map(|m| m.expect("split places every member"))
            .collect(),
    })
}

fn credit(s: &Source) -> Credit {
    Credit {
        title: s.title.clone(),
        author: s.author.clone(),
        license: s.spdx.clone(),
        url: Some(s.page.clone()).filter(|u| !u.is_empty()),
        note: s.credit.clone(),
    }
}

/// Every credited source, with the full notice text a licence (MIT) requires.
fn credits_txt(
    index: &Index,
    sources: &BTreeMap<String, Source>,
    inputs: &mut Inputs,
) -> Result<String, String> {
    let mut t = String::from("Sounds bundled with Honeydew: sources and licences.\n");
    for (key, c) in &index.credits {
        t += &format!(
            "\n== {} ==\nAuthor: {}\nLicence: {}\n",
            c.title, c.author, c.license
        );
        if let Some(url) = &c.url {
            t += &format!("Source: {url}\n");
        }
        if let Some(note) = &c.note {
            t += &format!("{note}\n");
        }
        if let Some(notice) = &sources[key].notice {
            let text = inputs.read_locked(key, notice)?;
            t += &format!("\n{}\n", String::from_utf8_lossy(&text).trim_end());
        }
    }
    Ok(t)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `path` and its precompressed `.gz` sibling.
fn with_gz(path: &Path, bytes: &[u8], gz: &[u8]) -> Result<(), String> {
    write(path, bytes)?;
    let mut name = path.as_os_str().to_owned();
    name.push(".gz");
    write(Path::new(&name), gz)
}
