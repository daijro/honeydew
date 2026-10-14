//! Procedural renders: one-shots, risers, FX and sliced loops synthesized at build time, so none of this DSP
//! enters the app. Recipes name a model (`render = "kick"`) with `[[variant]]` rows, or a kit whose pads each name one;
//! the renders then take the ordinary encode and pack path. Every number here is our own.
//!
//! Details:
//! - A one-shot is trimmed at −60 dB re its peak with a [`FADE_MS`] raised-cosine fade to 0
//!   (`dsp::trim_tail`), then set to peak at −1 dBFS. A loop keeps its exact length and is only set to −1 dBFS.
//! - Each render's noise and dither are seeded by the FNV-1a 32 hash of `<id>/<variant>` (`<id>/<pad>` for a kit pad,
//!   `<id>/<variant>/<voice index>` for a loop's voice), never 0.
//! - A kit pad's slot mirrors the code-made kits (`src/content/kits.rs`): name `<Kit> <Pad>`, folder
//!   `Built-in/Drumsets/<Kit>/<Pad>` (the pad name drawn large), Level from `level_db` on the Level law, Pan from
//!   `pan` (−1…1), and every pad whose name ends in `Hat` in cut group 1. Its `PTH1` is
//!   `@R@Built-in/Drumsets/<Kit file stem>/<Pad>.flac`.

mod drums;
mod fx;
mod loops;

use std::collections::BTreeMap;
use std::f32::consts::PI;

use honeydew::content::bundle::Kind;
use honeydew::flm::chunk::{ContentPath, ContentRoot};
use honeydew::flm::{read_drumset, write_drumset};
use honeydew::song::{DrumKit, SamplerSlot};
use honeydew_audio::flac_writer::FlacEncoder;
use honeydew_audio::sample::SampleMeta;

use crate::dsp;
use crate::dsp::resample::resample;
use crate::recipe::Recipe;

pub const RATE: u32 = 44_100;
const SR: f32 = RATE as f32;
/// −1 dBFS.
const PEAK: f32 = 0.891_250_9;
/// A one-shot's tail fade.
const FADE_MS: f32 = 10.0;

/// A model's parameters by name: its defaults, overridden by the recipe's.
type P = BTreeMap<&'static str, f32>;
/// A model renders mono or stereo audio at [`RATE`] from its parameters and seeded noise.
type Render = fn(&P, &mut Noise) -> Vec<Vec<f32>>;

/// A model's name, default parameters and render.
type Model = (&'static str, &'static [(&'static str, f32)], Render);
/// Member names relative to the entry base, and their bytes.
type Members = Vec<(String, Vec<u8>)>;

/// Every model.
const MODELS: &[Model] = &[
    ("kick", drums::KICK, drums::membrane),
    ("tom", drums::TOM, drums::membrane),
    ("snare", drums::SNARE, drums::snare),
    ("clap", drums::CLAP, drums::clap),
    ("hat", drums::HAT, drums::metal),
    ("cymbal", drums::CYMBAL, drums::metal),
    ("cowbell", drums::COWBELL, drums::cowbell),
    ("clave", drums::CLAVE, drums::clave),
    ("rim", drums::RIM, drums::rim),
    ("riser", fx::RISER, fx::sweep),
    ("downlifter", fx::DOWNLIFTER, fx::sweep),
    ("shepard", fx::SHEPARD, fx::shepard),
    ("impact", fx::IMPACT, fx::impact),
    ("whoosh", fx::WHOOSH, fx::whoosh),
    ("crackle", fx::CRACKLE, fx::crackle),
    ("tapestop", fx::TAPESTOP, fx::tapestop),
];

/// The members of a procedural recipe (a `samples` folder with `render`, or a kit whose pads `render`), in priority
/// order, or `None` for any other recipe.
pub fn members(r: &Recipe) -> Result<Option<(Members, usize)>, String> {
    let rendered = r.render.is_some() || r.pad.iter().any(|p| p.render.is_some());
    if !rendered {
        return Ok(None);
    }
    let err = |e: String| format!("{}: {e}", r.id);
    let mut out = Vec::new();
    match (r.kind, &r.render) {
        (Kind::Samples, Some(model)) if r.sample.is_empty() => {
            for v in &r.variant {
                let key = format!("{}/{}", r.id, v.name);
                // A variant may name its own model.
                let mut params = v.params.clone();
                let model = match params.remove("render") {
                    Some(toml::Value::String(m)) => m,
                    _ => model.clone(),
                };
                let (audio, meta) = if model == "loop" {
                    loops::render(&key, &params)
                } else {
                    one_shot(&model, &params, seed(&key)).map(|a| (a, SampleMeta::default()))
                }
                .map_err(|e| err(format!("{}: {e}", v.name)))?;
                out.push((format!("{}.flac", v.name), flac(&audio, seed(&key), &meta)));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (Kind::Kit, None) => {
            let file = r.path.rsplit('/').next().unwrap_or_default();
            let stem = file.strip_suffix(".tmpl").unwrap_or(file);
            let short = stem.strip_suffix(" Kit").unwrap_or(stem);
            let mut kit = DrumKit::default();
            for p in &r.pad {
                let model = (p.render.as_deref())
                    .ok_or_else(|| err(format!("{}: pad has no render", p.name)))?;
                let key = format!("{}/{}", r.id, p.name);
                let audio = one_shot(model, &p.params, seed(&key))
                    .map_err(|e| err(format!("{}: {e}", p.name)))?;
                let member = format!("{stem}/{}.flac", p.name);
                let mut slot = SamplerSlot {
                    name: format!("{short} {}", p.name),
                    folder: format!("Built-in/Drumsets/{stem}/{}", p.name),
                    cut_group: u8::from(p.name.ends_with("Hat")),
                    ..SamplerSlot::default()
                };
                // The Level law, dB = 66·v^0.6 − 60 (`src/content/kits.rs::level`), and a −1…1 pan to 0…1.
                slot.params[0] = ((p.level_db.unwrap_or(0.0) + 60.0) / 66.0).powf(1.0 / 0.6);
                slot.params[1] = 0.5 + 0.5 * p.pan.unwrap_or(0.0).clamp(-1.0, 1.0);
                slot.file.path = Some(ContentPath::Shorthand {
                    root: ContentRoot::App,
                    relative: format!("Built-in/Drumsets/{member}"),
                });
                kit.insert_slot(kit.slots.len(), slot);
                out.push((member, flac(&audio, seed(&key), &SampleMeta::default())));
            }
            let tmpl = write_drumset(&kit).map_err(|e| err(format!("{e:?}")))?;
            // The kit must load back with every pad's path, as the Drumsets picker will read it.
            let back = read_drumset(&tmpl).map_err(|e| err(format!("{e:?}")))?;
            let paths = |k: &DrumKit| {
                k.slots
                    .iter()
                    .map(|s| s.file.path.clone())
                    .collect::<Vec<_>>()
            };
            if paths(&back) != paths(&kit) {
                return Err(err("the kit does not read back with its pad paths".into()));
            }
            out.insert(0, (file.to_owned(), tmpl));
        }
        _ => {
            return Err(err(
                "`render` takes a samples folder of [[variant]] rows, or kit pads that each render"
                    .into(),
            ));
        }
    }
    Ok(Some((out, 0)))
}

/// One finished one-shot of `model`: rendered, trimmed with a fade and set to peak at −1 dBFS.
fn one_shot(model: &str, given: &toml::Table, seed: u32) -> Result<Vec<Vec<f32>>, String> {
    let (_, defaults, render) =
        (MODELS.iter().find(|(m, ..)| *m == model)).ok_or_else(|| format!("no model `{model}`"))?;
    let mut audio = render(&params(defaults, given)?, &mut Noise(seed));
    dsp::trim_tail(&mut audio, RATE, -60.0, FADE_MS, None);
    normalize(&mut audio)?;
    Ok(audio)
}

/// `defaults` with the recipe's numbers in place; a name the model lacks is an error.
fn params(defaults: &[(&'static str, f32)], given: &toml::Table) -> Result<P, String> {
    let mut p: P = defaults.iter().copied().collect();
    for (k, v) in given {
        let slot = (defaults.iter().find(|(d, _)| d == k))
            .map(|(d, _)| *d)
            .ok_or_else(|| format!("unknown parameter `{k}`"))?;
        let x = match v {
            toml::Value::Float(f) => *f as f32,
            toml::Value::Integer(i) => *i as f32,
            _ => return Err(format!("`{k}` is not a number")),
        };
        p.insert(slot, x);
    }
    Ok(p)
}

/// Scales `audio` to a true peak (4× oversampled) of −1 dBFS.
fn normalize(audio: &mut [Vec<f32>]) -> Result<(), String> {
    let peak = true_peak(audio);
    if !(peak > 0.0 && peak.is_finite()) {
        return Err("the render is silent or not finite".into());
    }
    audio.iter_mut().flatten().for_each(|x| *x *= PEAK / peak);
    Ok(())
}

/// The largest magnitude of `audio` upsampled 4× (`dsp::resample`), so inter-sample overs count.
fn true_peak(audio: &[Vec<f32>]) -> f32 {
    (audio.iter())
        .flat_map(|c| resample(c, RATE, 4 * RATE))
        .fold(0.0_f32, |m, x| m.max(x.abs()))
}

/// 16-bit FLAC with seeded TPDF dither.
fn flac(audio: &[Vec<f32>], seed: u32, meta: &SampleMeta) -> Vec<u8> {
    let mut enc = FlacEncoder::new(RATE, 16, audio.len());
    enc.push_i16(&dsp::dither(audio, seed.into()));
    enc.finish_with_meta(meta)
}

/// FNV-1a 32 of `key`, never 0 (xorshift's fixed point).
fn seed(key: &str) -> u32 {
    key.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    }) | 1
}

/// Frames in `s` seconds.
fn secs(s: f32) -> usize {
    (s.max(0.0) * SR) as usize
}

/// Seeded white noise in [−1, 1) (xorshift32).
pub struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / 2_147_483_648.0 - 1.0
    }
}

/// A state-variable filter (trapezoidal), retunable every sample.
#[derive(Default)]
struct Svf(f32, f32);

impl Svf {
    /// One sample at cutoff `hz` and resonance `q`: (low, band, high).
    fn tick(&mut self, x: f32, hz: f32, q: f32) -> (f32, f32, f32) {
        let g = (PI * hz.clamp(10.0, 0.45 * SR) / SR).tan();
        let k = 1.0 / q;
        let hp = (x - (k + g) * self.0 - self.1) / (1.0 + g * (g + k));
        let bp = g * hp + self.0;
        self.0 = g * hp + bp;
        let lp = g * bp + self.1;
        self.1 = g * bp + lp;
        (lp, bp, hp)
    }
}

/// A band-limited oscillator (polyBLEP), its phase in cycles.
#[derive(Default)]
struct Osc(f32);

impl Osc {
    /// A sawtooth sample at `hz`.
    fn saw(&mut self, hz: f32) -> f32 {
        let (t, dt) = self.step(hz);
        2.0 * t - 1.0 - blep(t, dt)
    }

    /// A square sample at `hz`.
    fn square(&mut self, hz: f32) -> f32 {
        let (t, dt) = self.step(hz);
        let naive = if t < 0.5 { 1.0 } else { -1.0 };
        naive + blep(t, dt) - blep((t + 0.5).fract(), dt)
    }

    /// The phase before this sample, and the phase step; advances.
    fn step(&mut self, hz: f32) -> (f32, f32) {
        let (t, dt) = (self.0, (hz / SR).clamp(1e-6, 0.5));
        self.0 = (t + dt).fract();
        (t, dt)
    }
}

/// The polyBLEP residual of a unit step at phase 0, for phase `t` and step `dt`.
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// An exponential decay of time constant `tau` after holding 1 for `hold` seconds.
fn env(t: f32, hold: f32, tau: f32) -> f32 {
    if t < hold {
        1.0
    } else {
        (-(t - hold) / tau).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use honeydew_audio::sample::decode;

    fn recipes() -> Vec<Recipe> {
        let content = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
        let all = crate::recipe::recipes(&content).unwrap();
        crate::gates::check(&all, &crate::recipe::sources(&content).unwrap()).unwrap();
        all.into_iter()
            .filter(|r| r.render.is_some() || r.pad.iter().any(|p| p.render.is_some()))
            .collect()
    }

    fn rendered(r: &Recipe) -> Vec<(String, Vec<u8>)> {
        members(r).unwrap().unwrap().0
    }

    #[test]
    fn a_procedural_render_is_bit_identical_twice() {
        let r = recipes()
            .into_iter()
            .find(|r| r.id == "proc-risers")
            .unwrap();
        assert!(rendered(&r) == rendered(&r));
    }

    #[test]
    fn procedural_one_shots_peak_at_minus_1_dbfs_and_fade_out_below_minus_60_db() {
        let mut n = 0;
        for r in recipes()
            .iter()
            .filter(|r| r.render.as_deref() != Some("loop"))
        {
            for (name, bytes) in rendered(r).iter().filter(|(n, _)| n.ends_with(".flac")) {
                let d = decode(bytes).unwrap();
                let chans: Vec<Vec<f32>> = [Some(&d.left), d.right.as_ref()]
                    .into_iter()
                    .flatten()
                    .map(|c| c.to_vec())
                    .collect();
                let db = 20.0 * true_peak(&chans).log10();
                assert!((db + 1.0).abs() <= 0.1, "{name}: true peak {db} dBFS");
                for c in &chans {
                    // The last frame is under −60 dB, and the whole tail fade under −50 dB: the sound decayed by
                    // itself before the trim, which only fades what is already quiet.
                    let fade = &c[c.len() - (FADE_MS * SR / 1000.0) as usize..];
                    assert!(
                        fade[fade.len() - 1].abs() < 1e-3 * PEAK,
                        "{name}: ends at {}",
                        fade[fade.len() - 1]
                    );
                    assert!(
                        fade.iter().all(|x| x.abs() < 3e-3 * PEAK),
                        "{name}: cut, not decayed"
                    );
                }
                n += 1;
            }
        }
        assert!(n > 100, "{n} one-shots");
    }

    #[test]
    fn a_procedural_loop_is_exactly_its_bars_long_with_cues_on_the_step_grid() {
        let r = recipes()
            .into_iter()
            .find(|r| r.render.as_deref() == Some("loop"))
            .unwrap();
        let loops = rendered(&r);
        for v in &r.variant {
            let bytes = &loops
                .iter()
                .find(|(n, _)| *n == format!("{}.flac", v.name))
                .unwrap()
                .1;
            let d = decode(bytes).unwrap();
            let (bpm, bars) = (
                v.params["bpm"].as_integer().unwrap() as usize,
                v.params["bars"].as_integer().unwrap() as usize,
            );
            assert_eq!(
                d.frames() * bpm,
                bars * 4 * 60 * RATE as usize,
                "{}",
                v.name
            );
            assert_eq!(d.meta.tempo, Some(bpm as f32));
            let steps = 16 * bars;
            assert!(!d.meta.cues.is_empty());
            for &cue in &d.meta.cues {
                let s = (cue as usize * steps + d.frames() / 2) / d.frames();
                assert_eq!(
                    cue as usize,
                    (s * d.frames() + steps / 2) / steps,
                    "{}: cue {cue} is off the grid",
                    v.name
                );
            }
        }
    }
}
