//! Licence, provenance and name checks. They read only `content/`, so `check` runs in gates without the
//! raw sources; `build` runs them first, and every input it opens passes [`input`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use honeydew::content::bundle::Kind;

use crate::recipe::{Recipe, Source};

const BANNED_HOSTS: [&str; 11] = [
    "splice.com",
    "cymatics.fm",
    "musicradar.com",
    "loopmasters.com",
    "producerloops.com",
    "looperman.com",
    "samplefocus.com",
    "sonniss.com",
    "99sounds.org",
    "bbcrewind.co.uk",
    "image-line.com",
];

/// The only inputs ever opened. Kontakt `.nki`/`.nkc`/`.nkr` files embed third-party scripts.
const INPUT_EXTS: [&str; 5] = ["sfz", "wav", "flac", "ogg", "sf2"];

/// Words a display name may not contain (sources may name them in credits).
const DENY_WORDS: [&str; 10] = [
    "fl",
    "rhodes",
    "wurlitzer",
    "steinway",
    "yamaha",
    "roland",
    "kontakt",
    "fender",
    "hammond",
    "leslie",
];
/// The same, matched with spaces and punctuation removed.
const DENY_RUNS: [&str; 5] = ["imageline", "flstudio", "tr808", "tr909", "tb303"];

/// A source's licence and hosts: CC0, PD-declared, MIT with a notice file, or CC-BY with a credit line; nothing from
/// a banned host.
pub fn licence(s: &Source) -> Result<(), String> {
    match s.spdx.as_str() {
        "CC0-1.0" | "LicenseRef-PD-declared" => {}
        "MIT" if s.notice.is_none() => {
            return Err(format!("{}: MIT needs a `notice` file", s.folder));
        }
        "MIT" => {}
        "CC-BY-3.0" | "CC-BY-4.0" if s.credit.is_none() => {
            return Err(format!("{}: {} needs a `credit` line", s.folder, s.spdx));
        }
        "CC-BY-3.0" | "CC-BY-4.0" => {}
        other => return Err(format!("{}: licence `{other}` is not allowed", s.folder)),
    }
    for url in std::iter::once(&s.page).chain(s.archives.iter().map(|a| &a.url)) {
        let host = host(url);
        if let Some(b) = BANNED_HOSTS
            .iter()
            .find(|b| host == **b || host.ends_with(&format!(".{b}")))
        {
            return Err(format!("{}: {url} is on a banned host ({b})", s.folder));
        }
    }
    Ok(())
}

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    host.split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// An input path: relative, inside its source folder, with a whitelisted extension.
pub fn input(rel: &str) -> Result<(), String> {
    within(rel, &INPUT_EXTS)
}

/// A file an SFZ reads as text: the `.sfz` itself or an `#include`, which may also be `.txt` (Splendid Grand's are).
/// No other input may be `.txt`.
pub fn sfz_text(rel: &str) -> Result<(), String> {
    within(rel, &["sfz", "txt"])
}

fn within(rel: &str, exts: &[&str]) -> Result<(), String> {
    let path = Path::new(rel);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if !ext.is_some_and(|e| exts.contains(&e.as_str())) {
        return Err(format!(
            "{rel}: not an allowed input (.{})",
            exts.join(", .")
        ));
    }
    if rel.contains('\\') || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("{rel}: must stay inside its source folder"));
    }
    Ok(())
}

/// A display name: none of the denylisted words.
pub fn name(display: &str) -> Result<(), String> {
    let lower = display.to_lowercase();
    let run: String = lower.chars().filter(|c| c.is_alphanumeric()).collect();
    let hit = lower
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| DENY_WORDS.contains(w));
    match hit.or_else(|| DENY_RUNS.iter().copied().find(|r| run.contains(r))) {
        Some(w) => Err(format!("`{display}` contains the denylisted `{w}`")),
        None => Ok(()),
    }
}

/// The path a picker lists the entry under, with plain folder and file names.
fn entry_path(kind: Kind, path: &str) -> Result<(), String> {
    let parts: Vec<&str> = path.split('/').collect();
    let shape = match kind {
        Kind::Program => {
            matches!(parts[..], ["Presets", "DW Sampler", c, n] if !c.is_empty() && n.len() > 4 && n.ends_with(".dwp"))
        }
        Kind::Kit => matches!(parts[..], ["Drumsets", n] if n.len() > 5 && n.ends_with(".tmpl")),
        Kind::Samples => {
            matches!(parts[..], ["Loops" | "Shapes", ""])
                || matches!(parts[..], ["Samples", c, ""] if !c.is_empty())
        }
    };
    if !shape || parts.iter().any(|p| !plain(p) && !p.is_empty()) {
        return Err(format!(
            "`{path}` is not a {kind:?} path (content-pipeline-plan §2.2)"
        ));
    }
    Ok(())
}

/// A path the app's `Index::parse` accepts. This mirrors `honeydew::content::bundle::safe` exactly: no `\`, `:` or
/// NUL, and no empty, `.` or `..` part. One name it refuses makes the app drop the whole bundle.
pub fn safe(path: &str) -> bool {
    !path.contains(['\\', ':', '\0'])
        && (path.split('/')).all(|part| !matches!(part, "" | "." | ".."))
}

/// A file or folder name: one [`safe`] part, without control characters.
fn plain(name: &str) -> bool {
    !name.contains('/') && !name.chars().any(char::is_control) && safe(name)
}

/// One recipe against `sources.json`.
pub fn recipe(r: &Recipe, sources: &BTreeMap<String, Source>) -> Result<(), String> {
    let id = &r.id;
    if id.is_empty() || !id.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-')) {
        return Err(format!("`{id}`: ids are [a-z0-9-]"));
    }
    entry_path(r.kind, &r.path).map_err(|e| format!("{id}: {e}"))?;
    for n in r
        .pad
        .iter()
        .map(|p| &p.name)
        .chain(r.sample.iter().map(|s| &s.name))
        .chain(r.variant.iter().map(|v| &v.name))
    {
        if !plain(n) {
            return Err(format!("{id}: `{n}` is not a plain name"));
        }
    }
    for n in r.display_names() {
        name(n).map_err(|e| format!("{id}: {e}"))?;
    }
    let mut samples = BTreeSet::new();
    let names = (r.sample.iter().map(|s| &s.name)).chain(r.variant.iter().map(|v| &v.name));
    if let Some(n) = names
        .chain(r.pad.iter().map(|p| &p.name))
        .find(|n| !samples.insert(*n))
    {
        return Err(format!("{id}: two samples are named `{n}`"));
    }
    let shape = match r.kind {
        Kind::Program => {
            usize::from(r.sfz.is_some())
                + usize::from(r.sf2.is_some())
                + usize::from(r.files.is_some())
                + usize::from(!r.zone.is_empty())
                == 1
        }
        Kind::Kit => !r.pad.is_empty(),
        Kind::Samples => !r.sample.is_empty() || (r.render.is_some() && !r.variant.is_empty()),
    };
    if !shape {
        return Err(format!(
            "{id}: a {:?} recipe needs one of `sfz`, `sf2`, `files`, `[[zone]]`; or `[[pad]]` or `[[sample]]` to match",
            r.kind
        ));
    }
    let mut licences = BTreeSet::new();
    for s in r.sources() {
        let source = sources
            .get(s)
            .ok_or_else(|| format!("{id}: source `{s}` is not in sources.json"))?;
        licence(source).map_err(|e| format!("{id}: {e}"))?;
        licences.insert(&source.spdx);
    }
    if licences.len() > 1 {
        return Err(format!(
            "{id}: sources under different licences ({licences:?}); one entry has one licence"
        ));
    }
    for (_, f) in r.files() {
        input(f).map_err(|e| format!("{id}: {e}"))?;
    }
    Ok(())
}

/// Every recipe, plus unique ids and paths.
pub fn check(recipes: &[Recipe], sources: &BTreeMap<String, Source>) -> Result<(), String> {
    let (mut ids, mut paths) = (BTreeSet::new(), BTreeSet::new());
    for r in recipes {
        recipe(r, sources)?;
        if !ids.insert(&r.id) || !paths.insert(&r.path) {
            return Err(format!("{}: duplicate id or path", r.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::Archive;

    fn source(spdx: &str, credit: bool, notice: bool) -> Source {
        Source {
            folder: "x".into(),
            title: "X".into(),
            page: "https://github.com/x/x".into(),
            author: "A".into(),
            spdx: spdx.into(),
            archives: vec![Archive {
                url: "https://example.org/x.zip".into(),
            }],
            credit: credit.then(|| "By A".into()),
            notice: notice.then(|| "LICENSE".into()),
        }
    }

    #[test]
    fn licence_gate_takes_cc0_pd_mit_with_notice_and_credited_cc_by_only() {
        for ok in [
            source("CC0-1.0", false, false),
            source("LicenseRef-PD-declared", false, false),
        ] {
            assert_eq!(licence(&ok), Ok(()));
        }
        assert_eq!(licence(&source("MIT", false, true)), Ok(()));
        assert!(
            licence(&source("MIT", false, false)).is_err(),
            "MIT without its notice"
        );
        assert_eq!(licence(&source("CC-BY-4.0", true, false)), Ok(()));
        assert!(
            licence(&source("CC-BY-3.0", false, false)).is_err(),
            "CC-BY without a credit"
        );
        for bad in [
            "GPL-3.0-or-later",
            "CC-BY-NC-4.0",
            "CC-BY-SA-4.0",
            "per-sound",
            "Sampling+",
        ] {
            assert!(licence(&source(bad, true, true)).is_err(), "{bad}");
        }
        let mut banned = source("CC0-1.0", false, false);
        banned.archives[0].url = "https://www.looperman.com/loops/1".into();
        assert!(licence(&banned).is_err(), "banned host");
    }

    #[test]
    fn inputs_are_whitelisted_and_stay_in_their_folder() {
        for ok in [
            "unpacked/a b/Piano.sfz",
            "s/kick.WAV",
            "x.flac",
            "y.ogg",
            "gm.sf2",
        ] {
            assert_eq!(input(ok), Ok(()), "{ok}");
        }
        for bad in [
            "cello.nki",
            "cello.nkc",
            "cello.nkr",
            "readme.txt",
            "noext",
            "../x.flac",
            "/abs/x.flac",
        ] {
            assert!(input(bad).is_err(), "{bad}");
        }
        // Only an SFZ's own reads take `.txt`.
        assert_eq!(sfz_text("Data/PP.txt"), Ok(()));
        assert!(sfz_text("Data/x.wav").is_err());
    }

    #[test]
    fn names_avoid_the_denylist_and_what_the_app_index_refuses() {
        for bad in ["FL Keys", "Image-Line Pad", "TR-808 Kit", "Stage Rhodes"] {
            assert!(name(bad).is_err(), "{bad}");
        }
        for ok in ["Flute", "Floor Tom", "Grand Piano"] {
            assert_eq!(name(ok), Ok(()), "{ok}");
        }
        // `bundle::safe` refuses these, and the app would drop the whole bundle over one.
        for bad in ["Lead: Bright", "a\\b", "nul\0", "..", ""] {
            assert!(!plain(bad), "{bad:?}");
        }
        assert!(plain("Lead - Bright"));
    }
}
