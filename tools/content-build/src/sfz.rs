//! A minimal SFZ reader: the `<control>`, `<global>`, `<master>`, `<group>` and `<region>` headers
//! with inheritance, `#define`, `#include`, `//` comments, `default_path`, and the opcodes the chosen sources map
//! with. Everything else is skipped; headers like `<curve>` and `<effect>` with their opcodes.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

/// One `<region>` with what it inherits from its `<global>`, `<master>` and `<group>`, and the SFZ defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// `default_path` + `sample` (backslashes read as slashes) under the root file's folder, with `.` and `..` resolved.
    pub sample: PathBuf,
    /// `lokey`, `hikey` and `pitch_keycenter` (default 0, 127, 60), each falling back to `key`. Numbers or note names
    /// (`c#4` = 61).
    pub lokey: u8,
    pub hikey: u8,
    pub pitch_keycenter: u8,
    /// Default 0 and 127.
    pub lovel: u8,
    pub hivel: u8,
    /// The round robin a region plays in its group's cycle; default 1.
    pub seq_position: u32,
    /// Random round robins: the region plays when a random number in [0, 1) falls in `lorand..hirand`.
    pub lorand: f32,
    pub hirand: f32,
    /// `attack` (default), `release`, `first`, `legato` or `release_key`.
    pub trigger: String,
    pub loop_mode: Option<String>,
    /// `sw_last`: the keyswitch that selects the region's articulation.
    pub sw_last: Option<u8>,
    /// `tune`: the sample's pitch correction in cents.
    pub tune: f32,
    pub loop_start: Option<u32>,
    pub loop_end: Option<u32>,
}

/// The regions of the SFZ file at `path` (relative to its source folder), in file order. `read` returns a file's text,
/// for the root file and each `#include` (a path relative to the root file's folder, as the sources expect), so the
/// caller decides what may be opened and hashes what was. Every path handed to `read` or put in a region has its `..`s
/// resolved, and one that climbs out of the source folder is an error. A `locc<N>`/`hicc<N>` pair drops a region when
/// controller N's value is outside it: from `cc`, else `<control>`'s `set_ccN` (or `set_hdccN` × 127), else 0.
/// Regions without a sample file (none, or a `*sine` generator) are left out, and so are CC-triggered ones.
pub fn parse(
    path: &Path,
    cc: &BTreeMap<u8, u8>,
    read: &mut impl FnMut(&Path) -> io::Result<String>,
) -> io::Result<Vec<Region>> {
    let path = normal(path)?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut text = String::new();
    expand(&path, dir, read, &mut BTreeMap::new(), 0, &mut text)?;

    // Words; one without `=` continues the value before it (a sample path with spaces).
    let text = text.replace('<', " <").replace('>', "> ");
    let mut tokens: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match tokens.last_mut() {
            Some(last)
                if !word.starts_with('<') && !word.contains('=') && !last.starts_with('<') =>
            {
                last.push(' ');
                last.push_str(word);
            }
            _ => tokens.push(word.to_owned()),
        }
    }

    // Opcodes per level: control, global, master, group, region. A header clears its level and those below it.
    let mut levels: [BTreeMap<String, String>; 5] = Default::default();
    let mut level = None;
    let mut regions = Vec::new();
    let mut flush = |levels: &[BTreeMap<String, String>; 5], level: Option<usize>| {
        if level == Some(4) {
            let merged = levels[1..]
                .iter()
                .flatten()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            regions.extend(region(&merged, &levels[0], dir, cc));
        }
    };
    for token in &tokens {
        if let Some(header) = token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
            flush(&levels, level);
            level = ["control", "global", "master", "group", "region"]
                .iter()
                .position(|h| *h == header);
            if let Some(l) = level.filter(|&l| l > 0) {
                levels[l..].iter_mut().for_each(BTreeMap::clear);
            }
        } else if let (Some(l), Some((key, value))) = (level, token.split_once('=')) {
            levels[l].insert(key.to_owned(), value.to_owned());
        }
    }
    flush(&levels, level);
    regions.into_iter().collect()
}

/// Appends `file`'s text to `out` with comments cut, `#include`s inlined and `$name`s replaced by their `#define`s.
fn expand(
    file: &Path,
    dir: &Path,
    read: &mut impl FnMut(&Path) -> io::Result<String>,
    defines: &mut BTreeMap<String, String>,
    depth: u8,
    out: &mut String,
) -> io::Result<()> {
    if depth > 16 {
        return Err(io::Error::other(format!(
            "{}: #include nested too deep",
            file.display()
        )));
    }
    for line in read(file)?.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if let Some(define) = line.strip_prefix("#define") {
            let mut words = define.split_whitespace();
            if let (Some(name), Some(value)) = (words.next(), words.next()) {
                defines.insert(name.to_owned(), value.to_owned());
            }
            continue;
        }
        let line = substitute(line, defines);
        if let Some(include) = line.strip_prefix("#include") {
            let name = include.trim().trim_matches('"').replace('\\', "/");
            expand(
                &normal(&dir.join(name))?,
                dir,
                read,
                defines,
                depth + 1,
                out,
            )?;
        } else {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Ok(())
}

/// `line` with each `$name` that is defined replaced by its value.
fn substitute(line: &str, defines: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let name_len = rest[at + 1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len() - at - 1);
        let name = &rest[at..at + 1 + name_len];
        out.push_str(defines.get(name).map_or(name, String::as_str));
        rest = &rest[at + 1 + name_len..];
    }
    out + rest
}

/// `path` with `.` and `..` resolved lexically. An error when it climbs out of the folder it is relative to, or is
/// absolute.
fn normal(path: &Path) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir if out.pop() => {}
            _ => {
                return Err(io::Error::other(format!(
                    "{}: leaves the source folder",
                    path.display()
                )));
            }
        }
    }
    Ok(out)
}

/// A region from its merged opcodes and the `<control>` ones; `None` without a sample file, when it is CC-triggered,
/// or when a CC range gates it out.
fn region(
    op: &BTreeMap<&str, &str>,
    control: &BTreeMap<String, String>,
    dir: &Path,
    cc: &BTreeMap<u8, u8>,
) -> Option<io::Result<Region>> {
    let sample = op.get("sample").filter(|s| !s.starts_with('*'))?;
    // `on_locc`/`on_hicc` (and the hdcc forms) start a region from a controller, and a -1 key range plays from no note.
    let triggered = op.keys().any(|k| k.starts_with("on_"))
        || ["key", "lokey", "hikey"]
            .iter()
            .any(|k| op.get(k) == Some(&"-1"));
    if triggered {
        return None;
    }
    let mut state = BTreeMap::new();
    for (k, v) in control {
        let (n, scale) = match (k.strip_prefix("set_cc"), k.strip_prefix("set_hdcc")) {
            (Some(n), _) => (n, 1.0),
            (_, Some(n)) => (n, 127.0),
            _ => continue,
        };
        if let (Ok(n), Ok(v)) = (n.parse::<u8>(), v.parse::<f32>()) {
            state.insert(n, v * scale);
        }
    }
    state.extend(cc.iter().map(|(&n, &v)| (n, f32::from(v))));
    let gated = op.iter().any(|(k, v)| {
        let (low, n) = match (k.strip_prefix("locc"), k.strip_prefix("hicc")) {
            (Some(n), _) => (true, n),
            (_, Some(n)) => (false, n),
            _ => return false,
        };
        let (Ok(n), Ok(bound)) = (n.parse::<u8>(), v.parse::<f32>()) else {
            return false;
        };
        let value = state.get(&n).copied().unwrap_or(0.0);
        if low { value < bound } else { value > bound }
    });
    if gated {
        return None;
    }
    let slashes = |p: &str| p.replace('\\', "/");
    let key = op.get("key").and_then(|v| note(v));
    let note_or = |k: &str, or: u8| op.get(k).and_then(|v| note(v)).or(key).unwrap_or(or);
    fn number<T: std::str::FromStr>(op: &BTreeMap<&str, &str>, key: &str) -> Option<T> {
        op.get(key)?.parse().ok()
    }
    let default_path = control.get("default_path").map_or("", String::as_str);
    let sample = normal(&dir.join(slashes(default_path)).join(slashes(sample)));
    Some(sample.map(|sample| Region {
        sample,
        lokey: note_or("lokey", 0),
        hikey: note_or("hikey", 127),
        pitch_keycenter: note_or("pitch_keycenter", 60),
        lovel: number(op, "lovel").unwrap_or(0),
        hivel: number(op, "hivel").unwrap_or(127),
        seq_position: number(op, "seq_position").unwrap_or(1),
        lorand: number(op, "lorand").unwrap_or(0.0),
        hirand: number(op, "hirand").unwrap_or(1.0),
        trigger: op.get("trigger").unwrap_or(&"attack").to_string(),
        loop_mode: op.get("loop_mode").map(|v| v.to_string()),
        sw_last: op.get("sw_last").and_then(|v| note(v)),
        tune: number(op, "tune").unwrap_or(0.0),
        loop_start: number(op, "loop_start"),
        loop_end: number(op, "loop_end"),
    }))
}

/// A MIDI note from a number or a name: a letter, `#` or `b`, and an octave where `c4` is 60.
pub(crate) fn note(v: &str) -> Option<u8> {
    if let Ok(n) = v.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let mut chars = v.chars();
    let mut semitone = match chars.next()?.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let octave = match rest.strip_prefix('#').or(rest.strip_prefix('b')) {
        Some(octave) => {
            semitone += if rest.starts_with('#') { 1 } else { -1 };
            octave
        }
        None => rest,
    };
    let n = 12 * (octave.parse::<i32>().ok()? + 1) + semitone;
    u8::try_from(n).ok().filter(|&n| n <= 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_includes_inheritance_cc_gates_and_round_robins() {
        let files = BTreeMap::from([
            (
                "kit/main.sfz",
                r#"// A kick in two round robins, a hat from an include, a pedal-down region and two CC-triggered ones.
#define $ROOT 36
<control> default_path=..\Samples\ set_cc64=127 set_hdcc7=0.5
<global> lovel=10 ampeg_release=0.3
<group> seq_length=2 key=$ROOT loop_mode=one_shot locc7=60 hicc7=70
<region> sample=kick rr1.wav seq_position=1
<region> sample=kick rr2.wav seq_position=2 hivel=90
#include "inc/hat.sfz"
<group> locc64=64 trigger=release
<region> sample=pedal.wav key=c#4 loop_start=10 loop_end=900
<group> lokey=-1 hikey=-1
<region> sample=thump.wav
<region> sample=lift.wav key=60 on_locc64=0 on_hicc64=1
"#,
            ),
            (
                "kit/inc/hat.sfz",
                "#define $HAT 42\n<group><region>sample=hat.wav key=$HAT lorand=0.5",
            ),
        ]);
        let mut read = |path: &Path| {
            let text = files
                .get(path.to_str().unwrap())
                .ok_or(io::ErrorKind::NotFound)?;
            Ok(text.to_string())
        };
        // The recipe's pedal-up overrides `set_cc64`; `set_hdcc7` (0.5 of 127) seeds the kick's CC 7 range.
        let up = parse(
            Path::new("kit/main.sfz"),
            &BTreeMap::from([(64, 0)]),
            &mut read,
        )
        .unwrap();
        let kick = Region {
            sample: "Samples/kick rr1.wav".into(),
            lokey: 36,
            hikey: 36,
            pitch_keycenter: 36,
            lovel: 10,
            hivel: 127,
            seq_position: 1,
            lorand: 0.0,
            hirand: 1.0,
            trigger: "attack".into(),
            loop_mode: Some("one_shot".into()),
            sw_last: None,
            tune: 0.0,
            loop_start: None,
            loop_end: None,
        };
        let kick2 = Region {
            sample: "Samples/kick rr2.wav".into(),
            hivel: 90,
            seq_position: 2,
            ..kick.clone()
        };
        // A new <group> drops the old group's opcodes but keeps the <global>'s.
        let hat = Region {
            sample: "Samples/hat.wav".into(),
            lokey: 42,
            hikey: 42,
            pitch_keycenter: 42,
            lorand: 0.5,
            loop_mode: None,
            ..kick.clone()
        };
        assert_eq!(up, [kick.clone(), kick2, hat]);

        // Pedal down from `set_cc64` alone: the gated region is in, with its note-name key. The CC-triggered regions
        // (keyed -1, or `on_locc`) never are.
        let down = parse(Path::new("kit/main.sfz"), &BTreeMap::new(), &mut read).unwrap();
        let pedal = Region {
            sample: "Samples/pedal.wav".into(),
            lokey: 61,
            hikey: 61,
            pitch_keycenter: 61,
            trigger: "release".into(),
            loop_mode: None,
            loop_start: Some(10),
            loop_end: Some(900),
            ..kick
        };
        assert_eq!(down.len(), 4);
        assert_eq!(down[3], pedal);

        // A sample path that climbs out of the source folder is an error, not a read outside it.
        let mut escape = |_: &Path| Ok("<region> sample=../../x.wav".to_owned());
        assert!(parse(Path::new("kit/x.sfz"), &BTreeMap::new(), &mut escape).is_err());
    }
}
