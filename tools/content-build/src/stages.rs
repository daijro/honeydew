//! The program and kit stages: SFZ regions or pad mic takes, through the DSP chain, into
//! 16-bit 44.1 kHz FLAC, plus the `.dwp` or `.tmpl` that plays them.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use honeydew::flm::chunk::{ContentPath, ContentRoot};
use honeydew::song::{DrumKit, SamplerSlot};
use honeydew_audio::dwp::{self, Zone};
use honeydew_audio::flac_writer::FlacEncoder;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::soundfont::{SoundFont, SoundFontPlayer};

use crate::build::{Inputs, Members};
use crate::dsp::{self, db, looping, peak, resample::resample};
use crate::recipe::{Recipe, Tail};
use crate::sfz::Region;

/// Every bundled file's rate (and 16-bit depth).
const RATE: u32 = 44_100;
/// The tail trim a recipe without `tail` gets.
const TAIL: Tail = Tail {
    floor_db: -60.0,
    fade_ms: 30.0,
    max_s: None,
};
/// Where `gain = "normalize"` puts a program's loudest sample: 6 dB under full scale, so chords have headroom.
/// Kit pads and one-shots stay at −1 dBFS.
const PROGRAM_PEAK_DB: f32 = -6.0;
/// The velocity a lite program's layer and the part order favour.
const FAVOURED_VELOCITY: usize = 90;
/// The velocity an `sf2` patch is rendered at: the app's default note velocity.
const RENDER_VELOCITY: f32 = 0.8;

/// One channel per `Vec`, at [`RATE`].
type Audio = Vec<Vec<f32>>;

/// A zone placed in a `.dwp`: its keys and velocities there.
type Placed<'a> = (&'a Point, (u8, u8), (u8, u8));

/// One zone of a program: where it plays and its audio.
struct Point {
    key: u8,
    keys: (u8, u8),
    vels: (u8, u8),
    /// Its velocity layer among the kept ones, and how many there are.
    layer: usize,
    layers: usize,
    file: String,
    audio: Audio,
    /// `[start, end)` in frames.
    looped: Option<(usize, usize)>,
}

/// A `program` recipe: its `.dwp`, an optional `lite.dwp`, then the zone files, in part order; and how many lead
/// the lite set.
///
/// Regions are the SFZ's attack regions of round robin `round_robin` (default 1; a random round robin keeps its first
/// slot) and, with `keyswitch`, of that articulation (`sw_last`), their keys and roots moved by `transpose`; an `sf2`
/// patch's renders at its `keys`, each over all keys and velocities; the `files` pattern's matches ([`pattern`]); or
/// the `[[zone]]`s. A sample point is a `pitch_keycenter`: `keys` keeps the first in `from..=to` and each next one at
/// least `every` semitones above the last kept, and the kept points split the source's key span at their midpoints.
/// At each point the regions by `lovel` are layers 1, 2 …, `velocity_layers` keeps those listed (all when empty), and
/// the kept ones split velocities 0–127 evenly. Each sample is decoded and pitched by its `tune` (or rendered, held at
/// velocity 0.8 for `loop.keep_s` + 1 s, else `tail.max_s`, else 10 s), resampled to 44.1 kHz and folded by
/// `channels`; then a `loop` with `find` is searched and baked, an SFZ `loop_continuous`/`loop_sustain` is kept as the
/// source has it (the sample cut after it), and anything else has its lead and tail trimmed. `gain = "normalize"`
/// scales the whole program so its loudest sample peaks at −6 dBFS; `even` first scales each key's layers alike so
/// that its loudest layer's RMS matches every other key's (for sources whose notes were recorded or attenuated
/// unevenly). `[[zone]]`s keep their own keys and velocities.
pub fn program(r: &Recipe, inputs: &mut Inputs) -> Result<(Members, usize), String> {
    let id = &r.id;
    let rr = u32::from(r.round_robin.unwrap_or(1));
    let (from, to) = r
        .keys
        .as_ref()
        .map_or((0, 127), |k| (k.from.unwrap_or(0), k.to.unwrap_or(127)));
    let mut font = None;
    let regions = match (&r.sfz, &r.sf2, &r.files) {
        (Some(sfz), ..) => {
            let cc = (r.cc.iter())
                .map(|(k, &v)| match k.parse::<u8>() {
                    Ok(k) => Ok((k, v)),
                    Err(_) => Err(format!("{id}: cc `{k}` is no controller number")),
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let mut regions = inputs.sfz(&r.source, sfz, &cc)?;
            let moved = |k: u8| (i16::from(k) + i16::from(r.transpose)).clamp(0, 127) as u8;
            for g in &mut regions {
                (g.lokey, g.hikey, g.pitch_keycenter) =
                    (moved(g.lokey), moved(g.hikey), moved(g.pitch_keycenter));
            }
            let articulation = |g: &Region| {
                r.keyswitch.is_none() || g.sw_last.is_none_or(|k| Some(k) == r.keyswitch)
            };
            regions.retain(|g| {
                g.trigger == "attack" && g.seq_position == rr && g.lorand <= 0.0 && articulation(g)
            });
            regions
        }
        (None, Some(sf2), _) => {
            let f = SoundFont::parse(inputs.read(&r.source, &sf2.file)?)
                .map_err(|e| format!("{id}: {}: {e:?}", sf2.file))?;
            let patch = (f.presets().iter())
                .position(|p| p.bank == 0 && p.preset == u16::from(sf2.preset))
                .ok_or_else(|| format!("{id}: no bank-0 preset {} in {}", sf2.preset, sf2.file))?;
            font = Some((Arc::new(f), patch));
            (sf2.keys.iter())
                .map(|&k| region(&sf2.file, k, (0, 127), (1, 127)))
                .collect()
        }
        (None, None, Some(files)) => pattern(r, files, rr, inputs)?,
        (None, None, None) => (r.zone.iter())
            .map(|z| {
                let keys = (z.lokey.unwrap_or(z.root), z.hikey.unwrap_or(z.root));
                let vels = (z.lovel.unwrap_or(0), z.hivel.unwrap_or(127));
                Region {
                    tune: z.tune,
                    ..region(&z.file, z.root, keys, vels)
                }
            })
            .collect(),
    };
    let explicit = !r.zone.is_empty();
    let mut points: BTreeMap<u8, Vec<Region>> = BTreeMap::new();
    for g in regions {
        points.entry(g.pitch_keycenter).or_default().push(g);
    }
    points.retain(|&k, _| (from..=to).contains(&k));
    // A point short of a kept layer is passed over: sources add odd keys on one layer only (Upright KW, Splendid).
    if let Some(&need) = r.velocity_layers.iter().max() {
        points.retain(|_, g| {
            let lovels: std::collections::BTreeSet<u8> = g.iter().map(|g| g.lovel).collect();
            lovels.len() >= usize::from(need)
        });
    }
    let regions = || points.values().flatten();
    let span = (
        regions().map(|g| g.lokey).min().unwrap_or(0),
        regions().map(|g| g.hikey).max().unwrap_or(127),
    );
    let keys = select(
        points.keys().copied(),
        r.keys.as_ref().map_or(1, |k| k.every),
    );
    if keys.is_empty() {
        return Err(format!("{id}: no sample point"));
    }
    let name = stem(&r.path);
    // ponytail: every zone's f32 audio stays in memory until the program's gain is known (about 200 MB for the
    // grand); take the peaks in a first pass and stream the second if a program outgrows that.
    let mut zones = Vec::new();
    for (&key, keys_range) in keys.iter().zip(spans(&keys, span)) {
        let mut layers = points[&key].clone();
        layers.sort_by_key(|g| g.lovel);
        layers.dedup_by_key(|g| g.lovel);
        let kept: Vec<&Region> = if r.velocity_layers.is_empty() {
            layers.iter().collect()
        } else {
            (r.velocity_layers.iter())
                .filter_map(|&l| layers.get(usize::from(l).checked_sub(1)?))
                .collect()
        };
        if !r.velocity_layers.is_empty() && kept.len() != r.velocity_layers.len() {
            return Err(format!(
                "{id}: key {key} has {} velocity layers, not all of {:?}",
                layers.len(),
                r.velocity_layers
            ));
        }
        for (layer, g) in kept.iter().enumerate() {
            let rel = g.sample.to_string_lossy();
            let (audio, rate) = match &font {
                Some((font, patch)) => (render(r, font, *patch, key), RATE),
                None => read(inputs, &r.source, &rel, g.tune)?,
            };
            let mut audio = channels(r, audio)?;
            let looped = shape(r, g, &mut audio, rate)?;
            zones.push(Point {
                key,
                keys: if explicit {
                    (g.lokey, g.hikey)
                } else {
                    keys_range
                },
                vels: if explicit {
                    (g.lovel, g.hivel)
                } else {
                    velocities(layer, kept.len())
                },
                layer,
                layers: kept.len(),
                file: format!("{} v{}.flac", note_name(key), layer + 1),
                audio,
                looped,
            });
        }
    }
    match r.gain.as_deref() {
        None => {}
        Some(gain @ ("normalize" | "even")) => {
            if gain == "even" {
                // Each key's layers alike, so that its loudest layer's RMS is the same at every key.
                for &key in &keys {
                    let at = |z: &&mut Point| z.key == key;
                    let loudest =
                        (zones.iter_mut().filter(at)).fold(0.0, |m, z| rms(&z.audio).max(m));
                    zones
                        .iter_mut()
                        .filter(at)
                        .for_each(|z| scale(&mut z.audio, 1.0 / loudest));
                }
            }
            let peak = zones.iter().map(|z| peak(&z.audio)).fold(0.0, f32::max);
            for z in &mut zones {
                scale(&mut z.audio, db(PROGRAM_PEAK_DB) / peak);
            }
        }
        Some(other) => return Err(format!("{id}: gain `{other}` is not `normalize` or `even`")),
    }

    let release = r.release.unwrap_or(Zone::default().floats[3]);
    let dwp = |zones: &[Placed]| {
        let rows: Vec<_> = (zones.iter())
            .map(|(z, keys, vels)| {
                let (loop_mode, (loop_start, loop_end)) = z
                    .looped
                    .map_or((0, (0, 0)), |(s, e)| (2, (s as u32, e as u32)));
                let zone = Zone {
                    root: z.key,
                    key_low: keys.0,
                    key_high: keys.1,
                    vel_low: vels.0,
                    vel_high: vels.1,
                    loop_mode,
                    loop_start,
                    loop_end,
                    floats: [0.0, 1.0, 1.0, release],
                    ..Zone::default()
                };
                let frames = z.audio[0].len() as u32;
                (zone, z.file.as_str(), frames, z.audio.len() == 2)
            })
            .collect();
        dwp::write(&rows)
    };
    let full: Vec<_> = zones.iter().map(|z| (z, z.keys, z.vels)).collect();
    let mut members = vec![(format!("{name}.dwp"), dwp(&full))];

    // The lite set: one layer, sparser points, over the same files.
    let mut lite_files = Vec::new();
    if let Some(lite) = &r.lite {
        let layer = if r.velocity_layers.is_empty() {
            usize::from(lite.layer).checked_sub(1)
        } else {
            r.velocity_layers.iter().position(|&l| l == lite.layer)
        };
        let layer = layer.ok_or_else(|| format!("{id}: lite layer {} is not kept", lite.layer))?;
        let lite_keys = select(keys.iter().copied(), lite.every);
        let lite_zones = (lite_keys.iter().zip(spans(&lite_keys, span)))
            .map(|(&key, keys)| {
                let z = zones.iter().find(|z| z.key == key && z.layer == layer);
                let z =
                    z.ok_or_else(|| format!("{id}: key {key} lacks lite layer {}", lite.layer))?;
                Ok((z, keys, (0, 127)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        members.push((format!("{name}/lite.dwp"), dwp(&lite_zones)));
        lite_files = lite_zones.iter().map(|(z, ..)| z.file.clone()).collect();
    }
    // Zone files: the lite set's, then by layer (the one at the favoured velocity, then louder, then softer), then key.
    let order = |z: &Point| {
        let favoured = FAVOURED_VELOCITY * z.layers / 128;
        let near = if z.layer >= favoured {
            (0, z.layer - favoured)
        } else {
            (1, favoured - z.layer)
        };
        (!lite_files.contains(&z.file), near, z.key)
    };
    zones.sort_by_key(order);
    let lite = members.len() + lite_files.len();
    for z in &zones {
        let member = format!("{name}/{}", z.file);
        let bytes = flac(&z.audio, seed(id, &member));
        members.push((member, bytes));
    }
    Ok((members, if r.lite.is_some() { lite } else { 0 }))
}

/// A `kit` recipe: its `.tmpl` and one FLAC per pad, in recipe order. Each pad sums its mic takes ([`dsp::premix`]),
/// folds by `channels`, has its lead and tail trimmed and peaks at −1 dBFS; the `.tmpl` sets its Level (`level_db`) and
/// Pan, puts every pad named `…Hat` in cut group 1, and points `PTH1` at the pad's file.
pub fn kit(r: &Recipe, inputs: &mut Inputs) -> Result<(Members, usize), String> {
    let name = stem(&r.path);
    let dir = Path::new(&r.path).parent().unwrap_or(Path::new(""));
    let mut kit = DrumKit::default();
    let mut members = Vec::new();
    for p in &r.pad {
        let source = p.source.as_deref().unwrap_or(&r.source);
        let takes = (p.mix.iter())
            .map(|m| read(inputs, source, &m.file, 0.0).map(|(audio, _)| audio))
            .collect::<Result<Vec<_>, _>>()?;
        let mics: Vec<_> = (takes.iter().zip(&p.mix))
            .map(|(channels, m)| dsp::Mic {
                channels,
                gain_db: m.gain_db,
                pan: m.pan.unwrap_or(0.0),
            })
            .collect();
        let mut audio = channels(r, dsp::premix(&mics))?;
        trim(r, &mut audio);
        let gain = db(-1.0) / peak(&audio);
        scale(&mut audio, gain);
        let member = format!("{name}/{}.flac", p.name);
        let bytes = flac(&audio, seed(&r.id, &member));
        members.push((member.clone(), bytes));

        // As the code-made kits' slots (`content::kits`): the hex pad reads `<Pad>` over `<Kit> <Pad>`.
        let short = name.strip_suffix(" Kit").unwrap_or(&name);
        let mut slot = SamplerSlot {
            name: format!("{short} {}", p.name),
            folder: format!("Built-in/{}/{name}/{}", dir.display(), p.name),
            cut_group: u8::from(p.name.ends_with("Hat")),
            ..SamplerSlot::default()
        };
        if let Some(db) = p.level_db {
            slot.params[0] = honeydew::content::kits::level(db);
        }
        slot.params[1] = 0.5 + 0.5 * p.pan.unwrap_or(0.0).clamp(-1.0, 1.0);
        slot.file.path = Some(ContentPath::Shorthand {
            root: ContentRoot::App,
            relative: format!("Built-in/{}/{member}", dir.display()),
        });
        kit.insert_slot(kit.slots.len(), slot);
    }
    let tmpl = honeydew::flm::write_drumset(&kit).map_err(|e| format!("{}: {e:?}", r.id))?;
    // The kit must load back through the app's reader with every pad's path, as the Drumsets picker will read it.
    let back = honeydew::flm::read_drumset(&tmpl).map_err(|e| format!("{}: {e:?}", r.id))?;
    let paths = |k: &DrumKit| {
        k.slots
            .iter()
            .map(|s| s.file.path.clone())
            .collect::<Vec<_>>()
    };
    if paths(&back) != paths(&kit) {
        return Err(format!(
            "{}: the kit does not read back with its pad paths",
            r.id
        ));
    }
    members.insert(0, (format!("{name}.tmpl"), tmpl));
    Ok((members, 0))
}

/// A `samples` recipe: one `<name>.flac` per `[[sample]]`, by name. A FLAC passes through as the source has it unless
/// the recipe sets `tail` or `channels`; anything else (WAV, Ogg) is decoded at 44.1 kHz, folded by `channels`, has its
/// lead and tail trimmed only when `tail` is set (a single cycle keeps every frame), and peaks at −1 dBFS as a pad does.
pub fn samples(r: &Recipe, inputs: &mut Inputs) -> Result<(Members, usize), String> {
    let mut members = Vec::new();
    for s in &r.sample {
        let source = s.source.as_deref().unwrap_or(&r.source);
        let member = format!("{}.flac", s.name);
        let flac_in = s.file.to_ascii_lowercase().ends_with(".flac");
        let bytes = if flac_in && r.tail.is_none() && r.channels.is_none() {
            inputs.read(source, &s.file)?
        } else {
            let mut audio = channels(r, read(inputs, source, &s.file, 0.0)?.0)?;
            if r.tail.is_some() {
                trim(r, &mut audio);
            }
            let gain = db(-1.0) / peak(&audio);
            scale(&mut audio, gain);
            flac(&audio, seed(&r.id, &member))
        };
        members.push((member, bytes));
    }
    members.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((members, 0))
}

/// A source file decoded, one channel per `Vec`, at [`RATE`] and `tune` cents up; and the rate it was taken to be at
/// (its own, `tune` cents up, to the hertz).
fn read(inputs: &mut Inputs, source: &str, rel: &str, tune: f32) -> Result<(Audio, u32), String> {
    let d = honeydew_audio::sample::decode(&inputs.read(source, rel)?)
        .map_err(|e| format!("{source}/{rel}: {e:?}"))?;
    let rate = (f64::from(d.sample_rate) * 2f64.powf(f64::from(tune) / 1200.0)).round() as u32;
    let channels = std::iter::once(&d.left).chain(d.right.as_ref());
    let audio = channels.map(|c| resample(c, rate, RATE)).collect();
    Ok((audio, rate))
}

/// Patch `patch` of `font` holding `key` at [`RENDER_VELOCITY`], centred, at [`RATE`]: for `loop.keep_s` + 1 s, else
/// `tail.max_s`, else 10 s.
fn render(r: &Recipe, font: &Arc<SoundFont>, patch: usize, key: u8) -> Audio {
    let secs = (r.looping.as_ref().map(|l| l.keep_s + 1.0))
        .or(r.tail.as_ref().and_then(|t| t.max_s))
        .unwrap_or(10.0);
    let n = (f64::from(RATE) * f64::from(secs)).round() as usize;
    let mut player = SoundFontPlayer::new(RATE as f32, Some((font.clone(), patch as i32)));
    player.note_on(key, RENDER_VELOCITY, 0.5);
    let (mut left, mut right) = (vec![0.0; n], vec![0.0; n]);
    let context = honeydew_audio::ProcessContext {
        sample_rate: RATE as f32,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };
    player.process(&mut left, &mut right, context);
    vec![left, right]
}

/// `audio` as the recipe's `channels` has it: `mono` (the mid), `stereo`, or `auto` ([`dsp::fold_mono`], the default).
fn channels(r: &Recipe, audio: Audio) -> Result<Audio, String> {
    match r.channels.as_deref() {
        None | Some("auto") => Ok(dsp::fold_mono(audio)),
        Some("stereo") => Ok(audio),
        Some("mono") if audio.len() == 2 => Ok(vec![
            (audio[0].iter().zip(&audio[1]))
                .map(|(l, r)| 0.5 * (l + r))
                .collect(),
        ]),
        Some("mono") => Ok(audio),
        Some(other) => Err(format!("{}: channels `{other}`", r.id)),
    }
}

/// The lead and tail trims, by the recipe's `tail`.
fn trim(r: &Recipe, audio: &mut Audio) {
    let t = r.tail.as_ref().unwrap_or(&TAIL);
    dsp::trim_lead(audio, RATE);
    dsp::trim_tail(audio, RATE, t.floor_db, t.fade_ms, t.max_s);
}

/// Loops or trims one zone's audio ([`program`]), its source at `rate`; its loop, `[start, end)`.
fn shape(
    r: &Recipe,
    g: &Region,
    audio: &mut Audio,
    rate: u32,
) -> Result<Option<(usize, usize)>, String> {
    let frames = |s: f32| (f64::from(RATE) * f64::from(s)).round() as usize;
    if let Some(l) = r.looping.as_ref().filter(|l| l.find) {
        dsp::trim_lead(audio, RATE);
        let xfade = frames(l.crossfade_ms / 1000.0);
        let (start, end) = looping::find_loop(audio, frames(l.min_s), frames(l.keep_s), xfade)
            .ok_or_else(|| format!("{}: no loop in {}", r.id, g.sample.display()))?;
        looping::bake_loop(audio, start, end, xfade);
        return Ok(Some((start, end)));
    }
    let sustained = matches!(
        g.loop_mode.as_deref(),
        Some("loop_continuous" | "loop_sustain")
    );
    if let (true, Some(start), Some(end)) = (sustained, g.loop_start, g.loop_end) {
        // The source's own loop, at our rate; nothing before it is trimmed, so its frames still hold.
        let at =
            |frame: u32| (f64::from(frame) * f64::from(RATE) / f64::from(rate)).round() as usize;
        let len = audio[0].len();
        let (start, end) = (at(start), at(end).min(len.saturating_sub(1)));
        if start + 2 > end {
            return Err(format!(
                "{}: loop {start}..{end} in {}",
                r.id,
                g.sample.display()
            ));
        }
        // SFZ ends are inclusive, but sources differ on whether the frame at `end` repeats the one at `start`: of
        // `end` and `end + 1`, the exclusive end whose straight-line continuation lands nearest the start frame.
        let bend = |e: usize| -> f32 {
            (audio.iter())
                .map(|c| (c[start] - (2.0 * c[e - 1] - c[e - 2])).abs())
                .sum()
        };
        let end = if bend(end + 1) < bend(end) {
            end + 1
        } else {
            end
        };
        audio.iter_mut().for_each(|c| c.truncate(end));
        return Ok(Some((start, end)));
    }
    trim(r, audio);
    Ok(None)
}

/// The regions of a `files` pattern (`<folder>/<name>`): each file in the folder whose name matches, at its `{note}`
/// plus `transpose`.
/// The `{layer}`s, in `layers` order (default: numbers in order, then text), are the regions' `lovel` order, and of
/// one note and layer's files the `{rr}` that is `rr` plays, else the lowest one.
fn pattern(r: &Recipe, files: &str, rr: u32, inputs: &mut Inputs) -> Result<Vec<Region>, String> {
    let id = &r.id;
    let (dir, name) = files.rsplit_once('/').unwrap_or(("", files));
    let bad = (name.split('{').skip(1))
        .map(|c| c.split_once('}').map_or(c, |(c, _)| c))
        .find(|c| !matches!(*c, "note" | "layer" | "rr"));
    if dir.contains(['{', '}']) || !name.contains("{note}") || bad.is_some() {
        return Err(format!(
            "{id}: `files` takes {{note}}, {{layer}} and {{rr}} in the file name only, {{note}} required"
        ));
    }
    let mut found = BTreeMap::new();
    for file in inputs.list(&r.source, dir)? {
        let Some(c) = capture(name, &file) else {
            continue;
        };
        let layer = c.get("layer").copied().unwrap_or_default().to_owned();
        if !r.layers.is_empty() && !r.layers.contains(&layer) {
            continue;
        }
        let note = (c.get("note").and_then(|n| crate::sfz::note(n)))
            .and_then(|n| n.checked_add_signed(r.transpose))
            .filter(|&n| n <= 127);
        let n = c.get("rr").map_or(Some(1), |n| n.parse::<u32>().ok());
        let (Some(note), Some(n)) = (note, n) else {
            continue;
        };
        if let Some(other) = found.insert((note, layer, n != rr, n), file.clone()) {
            return Err(format!(
                "{id}: {other} and {file} are one note, layer and rr"
            ));
        }
    }
    let mut order: Vec<&String> = found.keys().map(|(_, l, ..)| l).collect();
    if r.layers.is_empty() {
        order.sort_by_key(|l| l.parse::<u32>().map_err(|_| *l));
    } else {
        order = r.layers.iter().collect();
    }
    order.dedup();
    let mut out: Vec<Region> = Vec::new();
    for ((note, layer, ..), file) in &found {
        let lovel = order.iter().position(|l| *l == layer).unwrap_or(0) as u8;
        // `found` sorts the wanted rr first, then the lowest: keep a note and layer's first file.
        if (out.last()).is_none_or(|g| (g.pitch_keycenter, g.lovel) != (*note, lovel)) {
            let path = if dir.is_empty() {
                file.clone()
            } else {
                format!("{dir}/{file}")
            };
            out.push(region(&path, *note, (0, 127), (lovel, 127)));
        }
    }
    Ok(out)
}

/// `name`'s captures against the file-name pattern `pattern`, by placeholder: each as short as lets the rest match,
/// `{note}` a note name or number and `{rr}` digits.
fn capture<'p, 'n>(pattern: &'p str, name: &'n str) -> Option<BTreeMap<&'p str, &'n str>> {
    let Some((literal, rest)) = pattern.split_once('{') else {
        return (pattern == name).then(BTreeMap::new);
    };
    let (key, rest) = rest.split_once('}')?;
    let name = name.strip_prefix(literal)?;
    let fits = |value: &str| match key {
        "note" => crate::sfz::note(value).is_some(),
        "rr" => value.bytes().all(|b| b.is_ascii_digit()),
        _ => true,
    };
    (1..=name.len())
        .filter(|&i| name.is_char_boundary(i) && fits(&name[..i]))
        .find_map(|i| {
            let mut caps = capture(rest, &name[i..])?;
            caps.insert(key, &name[..i]);
            Some(caps)
        })
}

/// An attack region of `file` at `key`, over `keys` and `vels`.
fn region(file: &str, key: u8, keys: (u8, u8), vels: (u8, u8)) -> Region {
    Region {
        sample: file.into(),
        lokey: keys.0,
        hikey: keys.1,
        pitch_keycenter: key,
        lovel: vels.0,
        hivel: vels.1,
        seq_position: 1,
        lorand: 0.0,
        hirand: 1.0,
        trigger: "attack".into(),
        loop_mode: None,
        sw_last: None,
        tune: 0.0,
        loop_start: None,
        loop_end: None,
    }
}

/// Of the ascending `keys`, the first and each next one at least `every` above the last kept.
fn select(keys: impl Iterator<Item = u8>, every: u8) -> Vec<u8> {
    let mut kept: Vec<u8> = Vec::new();
    for k in keys {
        if kept
            .last()
            .is_none_or(|&last| k >= last.saturating_add(every.max(1)))
        {
            kept.push(k);
        }
    }
    kept
}

/// The key range each of the ascending `keys` plays: `span` split at the midpoints between neighbours.
fn spans(keys: &[u8], span: (u8, u8)) -> Vec<(u8, u8)> {
    (0..keys.len())
        .map(|i| {
            // Keys are MIDI notes (≤ 127), so a sum of two fits a u8.
            let low = match i.checked_sub(1) {
                Some(prev) => (keys[prev] + keys[i]) / 2 + 1,
                None => span.0.min(keys[0]),
            };
            let high = match keys.get(i + 1) {
                Some(&next) => (keys[i] + next) / 2,
                None => span.1.max(keys[i]),
            };
            (low, high)
        })
        .collect()
}

/// Layer `layer` of `layers`' share of velocities 0–127.
fn velocities(layer: usize, layers: usize) -> (u8, u8) {
    (
        (layer * 128 / layers) as u8,
        ((layer + 1) * 128 / layers - 1) as u8,
    )
}

/// `C4` for 60, `C#4` for 61.
fn note_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(key % 12)],
        i32::from(key / 12) - 1
    )
}

/// The file stem of a recipe `path`: its display name.
fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 16-bit 44.1 kHz FLAC of `audio`, dithered with `seed`.
fn flac(audio: &Audio, seed: u64) -> Vec<u8> {
    let q = dsp::dither(audio, seed);
    let mut enc = FlacEncoder::new(RATE, 16, q.len());
    enc.push_i16(&q);
    enc.finish()
}

/// The dither seed of a member: FNV-1a of `<id>/<member>`, so a rebuild dithers alike.
fn seed(id: &str, member: &str) -> u64 {
    format!("{id}/{member}")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
}

/// The RMS of all channels together.
fn rms(audio: &Audio) -> f32 {
    let n = audio.iter().map(Vec::len).sum::<usize>().max(1);
    let energy: f64 = audio.iter().flatten().map(|&x| f64::from(x).powi(2)).sum();
    (energy / n as f64).sqrt() as f32
}

fn scale(audio: &mut Audio, gain: f32) {
    if gain.is_finite() {
        audio.iter_mut().flatten().for_each(|x| *x *= gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_thin_to_every_n_and_split_the_span_at_midpoints() {
        // Salamander's minor-third grid, every 6: A0 C1 D#1 F#1 … keeps A0, D#1, A1 …
        let grid: Vec<u8> = (21..=108).step_by(3).collect();
        let kept = select(grid.iter().copied(), 6);
        assert_eq!(kept[..3], [21, 27, 33]);
        assert_eq!(spans(&kept[..3], (21, 35)), [(21, 24), (25, 30), (31, 35)]);
        // A point past the span still plays its own key; velocities tile 0–127.
        assert_eq!(spans(&[60], (62, 70)), [(60, 70)]);
        assert_eq!(
            (0..4).map(|l| velocities(l, 4)).collect::<Vec<_>>(),
            [(0, 31), (32, 63), (64, 95), (96, 127)]
        );
        assert_eq!((note_name(21), note_name(61)), ("A0".into(), "C#4".into()));
    }
}
