//! The instrument builder's mapping (issue #39, §3.3): audio file names split into tokens at a separator, each token
//! position given a [`Role`], parsed into key and velocity ranges, filled in where a name says nothing, and written as
//! an SFZ. Pure: names in, regions and text out. The builder panel is `ui::views::instrument_builder`.

/// The separators offered, besides a custom one.
pub const SEPARATORS: [&str; 5] = ["_", "-", ".", "@", "+"];

/// What a token position means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Ignore,
    /// The key the sample was recorded at; neighbouring roots share the keys between them.
    Root,
    /// The one key it plays, and its root.
    Key,
    LowKey,
    HighKey,
    /// `Vel1-100`, `64-127`; one value (`v64`) is the top of its layer.
    Velocity,
    LowVel,
    HighVel,
    RoundRobin,
    Group,
}

impl Role {
    pub const ALL: [Role; 10] = [
        Role::Ignore,
        Role::Root,
        Role::Key,
        Role::LowKey,
        Role::HighKey,
        Role::Velocity,
        Role::LowVel,
        Role::HighVel,
        Role::RoundRobin,
        Role::Group,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Role::Ignore => "Ignore",
            Role::Root => "Root key",
            Role::Key => "Single key",
            Role::LowKey => "Low key",
            Role::HighKey => "High key",
            Role::Velocity => "Velocity range",
            Role::LowVel => "Low velocity",
            Role::HighVel => "High velocity",
            Role::RoundRobin => "Round robin",
            Role::Group => "Group name",
        }
    }
}

/// Why a file's row is flagged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    /// The token at this position did not parse for its role (or the name has no token there): the file is left out.
    Unparsed(usize),
    /// Its keys and velocities overlap the region at this index, which is no round robin of it. It is kept.
    Overlaps(usize),
}

/// A file's region.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// The file's name.
    pub file: String,
    pub root: u8,
    pub keys: [u8; 2],
    pub vels: [u8; 2],
    /// Round robin: its position and the count in its zone, from 1; `None` alone in its zone.
    pub seq: Option<(u32, u32)>,
    pub group: String,
    pub flag: Option<Flag>,
}

/// The file name `name` without its extension.
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// The tokens of the file name `name` (without its extension) split at `separator`.
pub fn tokens<'a>(name: &'a str, separator: &str) -> Vec<&'a str> {
    match separator {
        "" => vec![stem(name)],
        separator => stem(name).split(separator).collect(),
    }
}

/// The separator `name` likely uses: the first of [`SEPARATORS`] in its stem, else `_`.
pub fn separator(name: &str) -> &'static str {
    let stem = stem(name);
    SEPARATORS
        .into_iter()
        .find(|s| stem.contains(s))
        .unwrap_or("_")
}

/// Roles guessed from `tokens`: the first note name is the root, `v64` or `Vel1-100` the velocities, `RR2` the round
/// robin; the rest are ignored.
pub fn guess(tokens: &[&str], c4: bool) -> Vec<Role> {
    let mut root = false;
    let numbered = |t: &str, prefix: &str| {
        let lower = t.to_ascii_lowercase();
        (lower.strip_prefix(prefix))
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    };
    let named = |t: &str| note(t, c4).is_some() && !t.bytes().all(|b| b.is_ascii_digit());
    (tokens.iter())
        .map(|&t| match t {
            t if numbered(t, "rr") || numbered(t, "seq") => Role::RoundRobin,
            t if numbered(t, "vel") || numbered(t, "v") => Role::Velocity,
            t if !root && named(t) => {
                root = true;
                Role::Root
            }
            _ => Role::Ignore,
        })
        .collect()
}

/// A MIDI note: a name (`C4`, `C#4`, `Db4`, any case; 60 is `C4` with `c4`, the app's `Set middle C to C4`, else
/// `C5`) or a number 0–127.
pub fn note(token: &str, c4: bool) -> Option<u8> {
    if token.bytes().all(|b| b.is_ascii_digit()) {
        return token.parse().ok().filter(|n| *n <= 127);
    }
    let key = honeydew_audio::sfz::note(&token.to_ascii_lowercase())? as i32 - 12 * i32::from(!c4);
    u8::try_from(key).ok().filter(|k| *k <= 127)
}

/// A velocity token's value: a range (`Vel1-100`, `64-127`), or one value (`v64`), the top of its layer.
#[derive(Clone, Copy)]
enum Vel {
    Range(u8, u8),
    Top(u8),
}

/// The number in a token, its letters around it dropped (`v64`, `RR2`).
fn number<T: std::str::FromStr>(token: &str) -> Option<T> {
    token
        .trim_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .ok()
}

fn velocity(token: &str) -> Option<u8> {
    number(token).filter(|v| *v <= 127)
}

fn velocities(token: &str) -> Option<Vel> {
    let token = token.trim_matches(|c: char| c.is_ascii_alphabetic());
    match token.split_once('-') {
        Some((low, high)) => {
            let (low, high) = (velocity(low)?, velocity(high)?);
            (low <= high).then_some(Vel::Range(low, high))
        }
        None => velocity(token).map(Vel::Top),
    }
}

/// What a file's tokens said.
#[derive(Default)]
struct Parsed {
    root: Option<u8>,
    key: Option<u8>,
    keys: [Option<u8>; 2],
    vel: Option<Vel>,
    vels: [Option<u8>; 2],
    rr: Option<u32>,
    group: String,
}

impl Parsed {
    /// Only a root: its keys are shared out between the neighbouring roots.
    fn spread(&self) -> bool {
        self.root.is_some() && self.key.is_none() && self.keys == [None, None]
    }
}

/// The file `name`'s tokens read by `roles`; `Err` with the position of one that does not parse, or is not there.
fn parse(name: &str, separator: &str, roles: &[Role], c4: bool) -> Result<Parsed, usize> {
    let tokens = tokens(name, separator);
    let mut p = Parsed::default();
    for (i, &role) in roles
        .iter()
        .enumerate()
        .filter(|(_, r)| **r != Role::Ignore)
    {
        let t = *tokens.get(i).ok_or(i)?;
        let note = || note(t, c4);
        let read = match role {
            Role::Ignore => Some(()),
            Role::Root => note().map(|k| p.root = Some(k)),
            Role::Key => note().map(|k| p.key = Some(k)),
            Role::LowKey => note().map(|k| p.keys[0] = Some(k)),
            Role::HighKey => note().map(|k| p.keys[1] = Some(k)),
            Role::Velocity => velocities(t).map(|v| p.vel = Some(v)),
            Role::LowVel => velocity(t).map(|v| p.vels[0] = Some(v)),
            Role::HighVel => velocity(t).map(|v| p.vels[1] = Some(v)),
            Role::RoundRobin => number(t).filter(|n| *n >= 1).map(|n| p.rr = Some(n)),
            Role::Group => {
                p.group = t.to_owned();
                Some(())
            }
        };
        read.ok_or(i)?;
    }
    Ok(p)
}

/// The span of `value` among the ascending `values` from `first` to `last`: from past the midpoint below it (the
/// previous value's, with `midpoint` false) to the midpoint above it (to the value itself).
fn share(values: &[u8], value: u8, (first, last): (u8, u8), midpoint: bool) -> [u8; 2] {
    let at = values.iter().position(|v| *v == value).unwrap_or_default();
    let mid = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
    let low = match at.checked_sub(1).map(|p| values[p]) {
        Some(prev) if midpoint => mid(prev, value) + 1,
        Some(prev) => prev + 1,
        None => first,
    };
    let high = match values.get(at + 1) {
        Some(&next) if midpoint => mid(value, next),
        Some(_) => value,
        None => last,
    };
    [low, high]
}

/// The regions of the files `names`, their tokens at `separator` read by `roles` (by position; past them, ignored);
/// `c4` is `Set middle C to C4`. A file with only a root shares the keyboard with the other roots of its group, split
/// at the midpoints; one with only a top velocity shares 1–127 with the other layers of its keys; one with neither
/// plays all keys at root 60, or all velocities. Round robins of one zone count from 1. A file whose token does not
/// parse is flagged and keeps those defaults; regions that overlap, round robins of one zone aside, are flagged too.
// ponytail: the overlap check is O(n²); a sweep by key if thousands of files ever map at once.
pub fn map(names: &[String], separator: &str, roles: &[Role], c4: bool) -> Vec<Region> {
    let parsed: Vec<_> = names
        .iter()
        .map(|n| parse(n, separator, roles, c4))
        .collect();
    let ok = || parsed.iter().flatten();
    let mut regions: Vec<Region> = names
        .iter()
        .zip(&parsed)
        .map(|(file, p)| {
            let mut region = Region {
                file: file.clone(),
                root: 60,
                keys: [0, 127],
                vels: [1, 127],
                seq: None,
                group: String::new(),
                flag: None,
            };
            let p = match p {
                Ok(p) => p,
                Err(at) => {
                    region.flag = Some(Flag::Unparsed(*at));
                    return region;
                }
            };
            region.group = p.group.clone();
            if let Some(key) = p.key {
                (region.root, region.keys) = (p.root.unwrap_or(key), [key, key]);
            } else if let [Some(low), Some(high)] =
                [p.keys[0].or(p.keys[1]), p.keys[1].or(p.keys[0])]
            {
                (region.root, region.keys) =
                    (p.root.unwrap_or(low), [low.min(high), low.max(high)]);
            } else if let Some(root) = p.root {
                let mut roots: Vec<u8> = (ok().filter(|o| o.spread() && o.group == p.group))
                    .filter_map(|o| o.root)
                    .collect();
                roots.sort();
                roots.dedup();
                (region.root, region.keys) = (root, share(&roots, root, (0, 127), true));
            }
            match (p.vel, p.vels) {
                (Some(Vel::Range(low, high)), _) => region.vels = [low, high],
                (_, [low, high]) if low.is_some() || high.is_some() => {
                    let (low, high) = (low.unwrap_or(1), high.unwrap_or(127));
                    region.vels = [low.min(high), low.max(high)];
                }
                _ => {}
            }
            region
        })
        .collect();
    // The layers of one group and keys that give only their top.
    for i in 0..regions.len() {
        let Ok(Parsed {
            vel: Some(Vel::Top(top)),
            ..
        }) = parsed[i]
        else {
            continue;
        };
        let same =
            |j: usize| regions[j].group == regions[i].group && regions[j].keys == regions[i].keys;
        let mut tops: Vec<u8> = (0..regions.len())
            .filter(|&j| same(j))
            .filter_map(|j| match parsed[j] {
                Ok(Parsed {
                    vel: Some(Vel::Top(top)),
                    ..
                }) => Some(top),
                _ => None,
            })
            .collect();
        tops.sort();
        tops.dedup();
        regions[i].vels = share(&tops, top, (1, 127), false);
    }
    // Round robins, by zone: group, keys and velocities.
    let zone = |r: &Region| (r.group.clone(), r.keys, r.vels);
    for i in 0..regions.len() {
        let Ok(Parsed { rr: Some(rr), .. }) = parsed[i] else {
            continue;
        };
        let mut rrs: Vec<u32> = (0..regions.len())
            .filter(|&j| regions[j].flag.is_none() && zone(&regions[j]) == zone(&regions[i]))
            .filter_map(|j| parsed[j].as_ref().ok()?.rr)
            .collect();
        rrs.sort();
        rrs.dedup();
        let at = rrs.iter().position(|r| *r == rr).unwrap_or_default();
        regions[i].seq = (rrs.len() > 1).then(|| (at as u32 + 1, rrs.len() as u32));
    }
    let meet = |a: [u8; 2], b: [u8; 2]| a[0] <= b[1] && b[0] <= a[1];
    let overlaps = |a: &Region, b: &Region| {
        let siblings = zone(a) == zone(b) && a.seq.is_some() && b.seq.is_some() && a.seq != b.seq;
        meet(a.keys, b.keys) && meet(a.vels, b.vels) && !siblings
    };
    let unparsed = |r: &Region| matches!(r.flag, Some(Flag::Unparsed(_)));
    let flags: Vec<Option<Flag>> = (0..regions.len())
        .map(|i| {
            let other = (0..regions.len()).find(|&j| {
                j != i
                    && !unparsed(&regions[i])
                    && !unparsed(&regions[j])
                    && overlaps(&regions[i], &regions[j])
            });
            other.map(Flag::Overlaps).or(regions[i].flag)
        })
        .collect();
    for (region, flag) in regions.iter_mut().zip(flags) {
        region.flag = flag;
    }
    regions
}

/// `base`, or the first of `base (2)`, `base (3)` … that `taken` does not hold.
pub fn free(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let names = std::iter::once(base.to_owned()).chain((2..).map(|n| format!("{base} ({n})")));
    names
        .into_iter()
        .find(|name| !taken(name))
        .expect("an unbounded range has a free number")
}

/// The file names `names` in one folder: each its own, a repeat of an earlier one (any case) `name (2).ext` and so on.
pub fn unique(names: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let (base, ext) = match name.rsplit_once('.') {
            Some((base, ext)) => (base, format!(".{ext}")),
            None => (name.as_str(), String::new()),
        };
        let taken = |n: &str| {
            let n = format!("{n}{ext}").to_lowercase();
            kept.iter().any(|k| k.to_lowercase() == n)
        };
        let free = free(base, taken);
        kept.push(format!("{free}{ext}"));
    }
    kept
}

/// The SFZ of `regions` (a flagged unparsed one left out), their samples in the folder `folder` beside it: a `<group>`
/// header, with its `group_label`, where the group changes, then a region a line, its `sample` last so that the path
/// runs to the line's end.
pub fn sfz(folder: &str, regions: &[Region]) -> String {
    use std::fmt::Write;
    let kept: Vec<&Region> = (regions.iter())
        .filter(|r| !matches!(r.flag, Some(Flag::Unparsed(_))))
        .collect();
    let app = crate::APP_NAME;
    let mut out = format!(
        "// {app} instrument: {} samples mapped from their names.\n",
        kept.len()
    );
    let mut group = None;
    for r in kept {
        if group != Some(&r.group) {
            group = Some(&r.group);
            out.push_str("\n<group>");
            if !r.group.is_empty() {
                let _ = write!(out, " group_label={}", r.group);
            }
            out.push('\n');
        }
        let [lokey, hikey] = r.keys;
        let [lovel, hivel] = r.vels;
        let _ = write!(
            out,
            "<region> lokey={lokey} hikey={hikey} pitch_keycenter={} lovel={lovel} hivel={hivel}",
            r.root
        );
        if let Some((position, length)) = r.seq {
            let _ = write!(out, " seq_length={length} seq_position={position}");
        }
        let _ = writeln!(out, " sample={folder}/{}", r.file);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(file: &str, root: u8, keys: [u8; 2], vels: [u8; 2]) -> Region {
        Region {
            file: file.into(),
            root,
            keys,
            vels,
            seq: None,
            group: String::new(),
            flag: None,
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn file_names_map_to_regions_by_their_token_roles() {
        use Role::*;
        assert_eq!(separator("Piano_C4-1.wav"), "_");
        assert_eq!(separator("Piano-C4.wav"), "-");
        assert_eq!(
            tokens("Piano_C4_Vel1-100.flac", "_"),
            ["Piano", "C4", "Vel1-100"]
        );
        // Note names in any case, with sharps or flats, and MIDI numbers; `Set middle C to C4` says which C is 60.
        let c4 = ["C4", "c4", "C#4", "Db4", "B3", "60", "127"].map(|t| note(t, true));
        assert_eq!(c4, [60, 60, 61, 61, 59, 60, 127].map(Some));
        assert_eq!((note("C5", false), note("C4", false)), (Some(60), Some(48)));
        assert_eq!(
            [note("128", true), note("X9", true), note("C", true)],
            [None; 3]
        );
        assert_eq!(
            guess(&["Piano", "C4", "v100", "RR1"], true),
            [Ignore, Root, Velocity, RoundRobin]
        );

        let files = names(&[
            "Piano_C4_Vel1-100_RR1.flac",
            "Piano_C4_Vel1-100_RR2.flac",
            "Piano_C4_Vel101-127_RR1.flac",
            "Piano_e4_v64_RR1.wav",
            "Piano_e4_v127_RR1.wav",
            "Piano_Db5_64-127_RR1.wav",
            "Piano_67_Vel1-127_RR1.wav",
            "Piano_X9_Vel1-127_RR1.wav",
            "Piano_C4_Vel50-110_RR1.flac",
        ]);
        let got = map(&files, "_", &[Ignore, Root, Velocity, RoundRobin], true);
        // Roots 60, 64, 67 and 73 share the keyboard at the midpoints; one value is the top of its layer.
        let rr = |r: Region, seq| Region {
            seq: Some(seq),
            ..r
        };
        let flagged = |r: Region, flag| Region {
            flag: Some(flag),
            ..r
        };
        let expected = [
            flagged(
                rr(region(&files[0], 60, [0, 62], [1, 100]), (1, 2)),
                Flag::Overlaps(8),
            ),
            flagged(
                rr(region(&files[1], 60, [0, 62], [1, 100]), (2, 2)),
                Flag::Overlaps(8),
            ),
            flagged(
                region(&files[2], 60, [0, 62], [101, 127]),
                Flag::Overlaps(8),
            ),
            region(&files[3], 64, [63, 65], [1, 64]),
            region(&files[4], 64, [63, 65], [65, 127]),
            region(&files[5], 73, [71, 127], [64, 127]),
            region(&files[6], 67, [66, 70], [1, 127]),
            flagged(region(&files[7], 60, [0, 127], [1, 127]), Flag::Unparsed(1)),
            flagged(region(&files[8], 60, [0, 62], [50, 110]), Flag::Overlaps(0)),
        ];
        assert_eq!(got, expected);

        // Explicit bounds, a single key, a group; no velocity is 1–127.
        let strings = map(
            &names(&["Strings_C3_B3_1_90.wav"]),
            "_",
            &[Group, LowKey, HighKey, LowVel, HighVel],
            true,
        );
        let group = |r: Region| Region {
            group: "Strings".into(),
            ..r
        };
        assert_eq!(
            strings,
            [group(region(
                "Strings_C3_B3_1_90.wav",
                48,
                [48, 59],
                [1, 90]
            ))]
        );
        let kick = map(&names(&["Kick-36.wav"]), "-", &[Ignore, Key], true);
        assert_eq!(kick, [region("Kick-36.wav", 36, [36, 36], [1, 127])]);
    }

    #[test]
    fn the_written_sfz_reads_back_as_the_same_regions() {
        use Role::*;
        let files = names(&[
            "Grand Piano_C4_v80_RR1.flac",
            "Grand Piano_C4_v80_RR2.flac",
            "Grand Piano_C4_v127_RR1.flac",
            "Grand Piano_G4_v127_RR1.flac",
        ]);
        let regions = map(&files, "_", &[Group, Root, Velocity, RoundRobin], true);
        let text = sfz("My Grand", &regions);
        let parsed = honeydew_audio::sfz::parse(&text, &|_| None);
        assert!(
            parsed.skipped.keys().all(|k| k == "group_label"),
            "{:?}",
            parsed.skipped
        );
        let back: Vec<_> = (parsed.regions.iter())
            .map(|r| {
                let seq = (r.seq_length > 1).then_some((r.seq_position, r.seq_length));
                let keys = r.keys.map(|k| k as u8);
                (r.sample.clone(), r.pitch_keycenter as u8, keys, r.vels, seq)
            })
            .collect();
        let want: Vec<_> = (regions.iter())
            .map(|r| {
                (
                    format!("My Grand/{}", r.file),
                    r.root,
                    r.keys,
                    r.vels,
                    r.seq,
                )
            })
            .collect();
        assert_eq!(back, want);
        assert_eq!(regions[0].seq, Some((1, 2)));
        // The folder keeps the names, a repeat (any case) numbered; the instrument's name is free with its folder.
        let picked = names(
            &["C4.wav", "Soft/c4.WAV", "C4.wav", "D4.wav"].map(|n| n.rsplit('/').next().unwrap()),
        );
        assert_eq!(
            unique(&picked),
            ["C4.wav", "c4 (2).WAV", "C4 (3).wav", "D4.wav"]
        );
        let taken = |n: &str| ["Piano", "Piano (2)"].contains(&n);
        assert_eq!(
            (free("Piano", taken), free("Keys", taken)),
            ("Piano (3)".into(), "Keys".into())
        );
    }
}
