//! `content-build verify`: checks a built output as served.

use std::collections::BTreeSet;
use std::path::Path;

use honeydew::content::bundle::{Index, IndexEntry, Kind, SCHEMA};

use crate::gates::safe;
use crate::pack::{self, CAP};
use crate::recipe::read;

/// The index budget, raw and gzipped.
const INDEX_RAW: usize = 512_000;
const INDEX_GZ: usize = 96_000;

pub fn verify(out: &Path, previous: Option<&Path>) -> Result<(), String> {
    verify_capped(out, previous, CAP)
}

/// Every problem, one per line: parts over `cap` raw or gzipped, bad hashes, names or member ranges, the index over
/// budget, a bundled stem that shadows a code-made Built-in, and paths of `previous` neither kept nor aliased.
pub fn verify_capped(out: &Path, previous: Option<&Path>, cap: usize) -> Result<(), String> {
    let mut bad = Vec::new();
    let raw = read(&out.join("index.json"))?;
    let gz = read(&out.join("index.json.gz"))?;
    if pack::gunzip(&gz).as_ref() != Some(&raw) {
        bad.push("index.json.gz: does not inflate to index.json".into());
    }
    if raw.len() > INDEX_RAW || gz.len() > INDEX_GZ {
        bad.push(format!(
            "index.json: {} bytes raw, {} gzipped (budget {INDEX_RAW} / {INDEX_GZ})",
            raw.len(),
            gz.len()
        ));
    }
    let index = parse(&raw)?;
    // The names the app's `Index::parse` checks, by the same rule (`gates::safe` mirrors `bundle::safe`).
    for (old, new) in &index.aliases {
        if !safe(old) || !safe(new) {
            bad.push(format!("alias {old:?} -> {new:?}: badly named"));
        }
    }
    let mut ids = BTreeSet::new();
    for e in &index.entries {
        if !ids.insert(&e.id) {
            bad.push(format!("{}: duplicate id", e.id));
        }
        let path = match e.kind {
            Kind::Samples => e.path.strip_suffix('/'),
            Kind::Program | Kind::Kit => Some(e.path.as_str()),
        };
        if !path.is_some_and(safe) || !e.lite.as_deref().is_none_or(safe) {
            bad.push(format!("{}: path or lite badly named", e.id));
        }
        for p in &e.parts {
            bad.extend(
                part(out, &p.file, p.bytes, &p.sha256, cap)
                    .err()
                    .map(|m| format!("{}: {m}", e.id)),
            );
        }
        for m in &e.members {
            let fits = e
                .parts
                .get(m.1)
                .is_some_and(|p| m.2 % 512 == 0 && m.2 + m.3 <= p.bytes);
            if !fits || !safe(&m.0) {
                bad.push(format!(
                    "{}: member {:?} is outside its part or badly named",
                    e.id, m.0
                ));
            }
            let stem = format!("Built-in/{}{}", base(e), m.0);
            if let Some(stem) = stem.strip_suffix(".flac")
                && honeydew_audio::builtin_drums::builtin(stem).is_some()
            {
                bad.push(format!(
                    "{}: {stem} shadows a code-made Built-in sound",
                    e.id
                ));
            }
        }
    }
    if let Some(prev) = previous {
        let old = parse(&read(prev)?)?;
        let now: BTreeSet<String> = index.entries.iter().flat_map(stable).collect();
        for path in old.entries.iter().flat_map(stable) {
            let kept =
                now.contains(&path) || index.aliases.get(&path).is_some_and(|to| now.contains(to));
            if !kept {
                bad.push(format!(
                    "{path}: in the previous index but neither kept nor aliased"
                ));
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join("\n"))
    }
}

fn parse(raw: &[u8]) -> Result<Index, String> {
    let index: Index = serde_json::from_slice(raw).map_err(|e| format!("index.json: {e}"))?;
    if index.schema != SCHEMA {
        return Err(format!(
            "index.json: schema {} (this tool writes {SCHEMA})",
            index.schema
        ));
    }
    Ok(index)
}

/// One part: named by its hash, the size and SHA-256 the index says, at most `cap` raw and gzipped, and its `.gz`
/// inflating to it.
fn part(out: &Path, file: &str, bytes: u64, sha: &str, cap: usize) -> Result<(), String> {
    let hex16 = file
        .strip_prefix("p/")
        .and_then(|f| f.strip_suffix(".tar"))
        .filter(|h| h.len() == 16);
    let hex16 = hex16.ok_or_else(|| format!("{file}: not p/<hash16>.tar"))?;
    let raw = read(&out.join(file))?;
    let gz = read(&out.join(format!("{file}.gz")))?;
    if raw.len() > cap || gz.len() > cap {
        return Err(format!(
            "{file}: {} bytes raw, {} gzipped, over the {cap}-byte cap",
            raw.len(),
            gz.len()
        ));
    }
    let digest = pack::sha256(&raw);
    if raw.len() as u64 != bytes
        || pack::base64(&digest) != sha
        || pack::hex(&digest)[..16] != *hex16
    {
        return Err(format!("{file}: size or hash differs from the index"));
    }
    if pack::gunzip(&gz).as_ref() != Some(&raw) {
        return Err(format!("{file}.gz: does not inflate to {file}"));
    }
    Ok(())
}

/// The folder member names are relative to, below `Built-in/`, with a trailing `/` (or empty).
fn base(e: &IndexEntry) -> &str {
    match e.kind {
        Kind::Samples => &e.path,
        Kind::Program | Kind::Kit => e.path.rfind('/').map_or("", |i| &e.path[..=i]),
    }
}

/// The paths songs store, which a release must keep or alias: a program's file; a kit's file and pads; a
/// folder's files.
fn stable(e: &IndexEntry) -> Vec<String> {
    let members = e.members.iter().map(|m| format!("{}{}", base(e), m.0));
    match e.kind {
        Kind::Program => vec![e.path.clone()],
        Kind::Kit => members
            .filter(|p| p.ends_with(".flac"))
            .chain([e.path.clone()])
            .collect(),
        Kind::Samples => members.collect(),
    }
}
