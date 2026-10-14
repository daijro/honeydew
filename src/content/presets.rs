//! Our instrument preset banks, `Presets/<module folder>/<Category>/<Name>.flmpst`: our own sound design and names,
//! made in code. A bank's category is its folder; a bank without categories (Transistor Bass) lists its presets right
//! in its module folder.
//!
//! A [`Preset`] names its params as the registry does ([`crate::params::module_spec`]): by automation name, or `#n` for
//! a param with no name or one it shares (SuperSaw's two `Osc. Pulse Width`, its unnamed FX Gain `#53`, Transistor
//! Bass's DISTORTION and PULSE WIDTH pages); the common `Post Gain` by its name. Its `settings` are the module's `CSTM`
//! settings by `#n` ([`cstm_from_settings`]). Each file is [`flm::write_preset`] of [`module`], which names the module
//! as a pick from the factory names it, so a template's module ([`find`]) and a pick of the same preset are equal.
//!
//! **Values.** Menu values sit mid-step (MiniSynth `OSC Kind` 0.25 is PULSE, trunc(9v) = 2), clear of the step edges
//! where f32 rounding could tip them. A name that resolves to no param, or to more than one, is a bank error the tests
//! catch; [`module`] skips it.
//!
//! **Levels.** A single C3 peaks at −18 dBFS or more, a C minor triad at −3 dBFS or less, and a C major triad and a C
//! minor seventh at −2 dBFS or less (`tests/content_presets.rs`). Chord presets (keys, bells, pads, plucks, stabs) trim
//! with `Post Gain`, which leaves the sound alone (MiniSynth has no level of its own). A saved Post Gain holds from a
//! module's first sample, so a trimmed preset keeps an instant attack where it is musical; the MiniSynth keys' few-ms
//! attack is their soft onset, not a workaround. Transistor Bass, whose chain ends in ×0.25, makes its level up on the
//! DISTORTION page: on, Drive (`#9`) 0 unless the preset drives it, Drive Vol. (`#10`) as makeup, so the bank peaks at
//! −9 … −11 dBFS like the other basses.
//!
//! GMSynth ([`gm_synth`]) and 3x Osc ([`three_osc`]) keep their banks in their own files.

use std::collections::HashMap;
use std::iter::once;
use std::path::Path;

mod gm_synth;
mod three_osc;

use super::{Entry, PRESETS, ROOT};
use crate::flm;
use crate::panels::cstm::cstm_from_settings;
use crate::params::{COMMON_PARAMS, ModuleSpec, module_spec};
use crate::project_files::{PRESET_EXT, preset_folder};
use crate::song::ModuleInstance;

/// One preset of ours.
pub struct Preset {
    /// Its folder in the bank, `""` for none.
    pub category: &'static str,
    pub name: &'static str,
    /// Normalized values by param name (or `#n`); the rest keep the module's defaults.
    pub params: &'static [(&'static str, f32)],
    /// `CSTM` settings by `#n`.
    pub settings: &'static [(u32, f32)],
}

/// Every bank: a module type and its presets.
fn banks() -> impl Iterator<Item = (u16, &'static [Preset])> {
    once((0, MINISYNTH))
        .chain(once((23, SUPERSAW)))
        .chain(once((19, TRANSISTOR_BASS)))
        .chain(once((2, gm_synth::BANK)))
        .chain(once((39, three_osc::BANK)))
}

/// The param `name` names in `spec`, as [`ModuleInstance::param`] indexes it: `#n` (digits only), or the one param or
/// common param (negative) of that name. `None` for Enable (param 0, which stays on), an empty name, no param or
/// several.
fn resolve(spec: &ModuleSpec, name: &str) -> Option<i32> {
    if let Some(n) = name.strip_prefix('#') {
        let digits = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
        let index = digits.then(|| n.parse::<usize>().ok()).flatten()?;
        return (1..spec.params.len())
            .contains(&index)
            .then_some(index as i32);
    }
    let own = (spec.params.iter().enumerate().skip(1)).map(|(i, p)| (i as i32, p.name));
    let common = (COMMON_PARAMS.iter().enumerate()).map(|(k, p)| (-1 - k as i32, p.name));
    let mut named = own
        .chain(common)
        .filter(|&(_, n)| !name.is_empty() && n == name);
    let (index, _) = named.next()?;
    named.next().is_none().then_some(index)
}

/// `preset`'s path below [`ROOT`]: `Presets/<module folder>/[<Category>/]<Name>.flmpst`.
fn rel(type_id: u16, preset: &Preset) -> Option<String> {
    let folder = preset_folder(type_id)?;
    let category = match preset.category {
        "" => String::new(),
        category => format!("{category}/"),
    };
    Some(format!(
        "{PRESETS}/{folder}/{category}{}.{PRESET_EXT}",
        preset.name
    ))
}

/// The module `preset` of module type `type_id` is, instance id 0, named as a pick of it from the factory.
pub fn module(type_id: u16, preset: &Preset) -> ModuleInstance {
    let mut module = ModuleInstance::new(type_id, 0);
    if let Some(spec) = module_spec(type_id) {
        for &(name, value) in preset.params {
            if let Some(param) = resolve(spec, name).and_then(|i| module.param_mut(i)) {
                *param = value;
            }
        }
    }
    if !preset.settings.is_empty() {
        let settings: HashMap<u32, f32> = preset.settings.iter().copied().collect();
        module.file.custom_data = cstm_from_settings(type_id, &settings, None);
    }
    if let Some(rel) = rel(type_id, preset) {
        module.name_preset(&Path::new(ROOT).join(rel), Path::new(ROOT));
        // As a pick names it (`ProjectFiles::name_module`): the source path relative to its root.
        module.file.source_path = module.file.preset_path.clone();
    }
    module
}

/// Our preset `name` for module type `type_id`, as a module ([`module`]). The templates build with it.
pub fn find(type_id: u16, name: &str) -> Option<ModuleInstance> {
    let (_, bank) = banks().find(|&(t, _)| t == type_id)?;
    let preset = bank.iter().find(|p| p.name == name)?;
    Some(module(type_id, preset))
}

pub(super) fn entries() -> Vec<Entry> {
    let presets = banks().flat_map(|(t, bank)| bank.iter().map(move |p| (t, p)));
    presets
        .filter_map(|(t, preset)| {
            let bytes =
                move || flm::write_preset(&module(t, preset)).expect("a bank preset writes");
            Some(Entry::new(rel(t, preset)?, bytes))
        })
        .collect()
}

/// A preset without `CSTM` settings.
const fn p(
    category: &'static str,
    name: &'static str,
    params: &'static [(&'static str, f32)],
) -> Preset {
    Preset {
        category,
        name,
        params,
        settings: &[],
    }
}

// ---- MiniSynth (0) ----
// `Voices Mode` 0.5 MONO, 1 LEAD. `OSC Kind` by trunc(9v): 0 SAW, 0.15 SAW+SAW, 0.25 PULSE, 0.37 SQR+SAW, 0.48
// SQR+SQR, 0.6 SUPERSAW, 0.7 BELL, 0.82 DEAF SAW, 0.93 SAW SPREAD, 1 SAW SPREAD 2. `OSC Mod` 0.5 is no interval (a
// 7.5 cent detune), 0.795 a fifth up, 1 an octave up; a pulse's width is 1 − v.

const MINISYNTH: &[Preset] = &[
    p(
        "Bass",
        "Rubber Floor",
        &[
            ("Voices Mode", 0.5),
            ("Slide Time", 0.15),
            ("OSC Kind", 0.25),
            ("OSC Mod", 0.62),
            ("FLT Freq", 0.36),
            ("FLT Peak", 0.35),
            ("FLT Kbd", 0.3),
            ("EGF Dec", 0.3),
            ("EGF Sus", 0.0),
            ("EGF Rel", 0.3),
            ("EGF Amnt", 0.68),
            ("AMP Rel", 0.3),
        ],
    ),
    p(
        "Bass",
        "Velvet Sub",
        &[
            ("Voices Mode", 0.5),
            ("Transpose", 0.0),
            ("FLT Freq", 0.4),
            ("FLT Peak", 0.1),
            ("FLT Kbd", 0.5),
            ("EGF Dec", 0.45),
            ("EGF Sus", 0.3),
            ("EGF Amnt", 0.6),
            ("AMP Rel", 0.35),
        ],
    ),
    p(
        "Bass",
        "Gravel Bass",
        &[
            ("Voices Mode", 0.5),
            ("OSC Kind", 0.37),
            ("OSC Mod", 1.0),
            ("FLT Freq", 0.45),
            ("FLT Peak", 0.5),
            ("EGF Dec", 0.22),
            ("EGF Sus", 0.05),
            ("EGF Amnt", 0.72),
            ("AMP Dec", 0.55),
            ("AMP Sus", 0.5),
            ("AMP Rel", 0.25),
            ("DIST", 1.0),
            ("Overdrive", 0.35),
        ],
    ),
    p(
        "Keys",
        "Glass Tines",
        &[
            ("OSC Kind", 0.7),
            ("OSC Mod", 0.35),
            ("FLT Freq", 0.62),
            ("FLT Vel", 0.4),
            ("EGF Dec", 0.45),
            ("EGF Sus", 0.25),
            ("EGF Amnt", 0.62),
            ("AMP Dec", 0.62),
            ("AMP Sus", 0.0),
            ("AMP Rel", 0.55),
            ("CHORUS", 1.0),
            ("CHORUS ModRate", 0.3),
            ("CHORUS ModDepth", 0.5),
            ("CHORUS Mix", 0.5),
            ("Post Gain", 0.375),
            ("AMP Atk", 0.08),
        ],
    ),
    p(
        "Keys",
        "Felt Keys",
        &[
            ("OSC Kind", 0.25),
            ("OSC Mod", 0.55),
            ("FLT Freq", 0.48),
            ("FLT Kbd", 0.5),
            ("FLT Vel", 0.5),
            ("EGF Dec", 0.42),
            ("EGF Sus", 0.15),
            ("EGF Amnt", 0.66),
            ("AMP Dec", 0.58),
            ("AMP Sus", 0.15),
            ("AMP Rel", 0.5),
            ("Post Gain", 0.29),
            ("AMP Atk", 0.1),
        ],
    ),
    p(
        "Keys",
        "Reed Organ",
        &[
            ("OSC Kind", 0.48),
            ("OSC Mod", 0.795),
            ("FLT Freq", 0.55),
            ("AMP Atk", 0.1),
            ("AMP Rel", 0.4),
            ("PHASER", 1.0),
            ("PHASER ModSpeed", 0.3),
            ("PHASER ModWidth", 0.4),
            ("PHASER Mix", 0.35),
            ("Post Gain", 0.24),
        ],
    ),
    p(
        "Lead",
        "Neon Ribbon",
        &[
            ("Voices Mode", 1.0),
            ("Slide Time", 0.3),
            ("OSC Kind", 0.15),
            ("OSC Mod", 0.5),
            ("FLT Freq", 0.68),
            ("FLT Peak", 0.3),
            ("FLT Kbd", 0.4),
            ("EGF Dec", 0.4),
            ("EGF Sus", 0.5),
            ("EGF Amnt", 0.6),
            ("LFO Dest", 0.5),
            ("LFO Rate", 0.15),
            ("LFO Amnt", 0.57),
            ("AMP Rel", 0.35),
            ("DELAY", 1.0),
            ("DELAY Time", 0.4),
            ("DELAY Fdbk", 0.35),
            ("DELAY Filter", 0.6),
            ("DELAY Mix", 0.45),
        ],
    ),
    p(
        "Lead",
        "Hollow Reed",
        &[
            ("Voices Mode", 0.5),
            ("Slide Time", 0.2),
            ("OSC Kind", 0.25),
            ("OSC Mod", 0.5),
            ("OSC Noise", 0.2),
            ("FLT Freq", 0.6),
            ("FLT Peak", 0.2),
            ("FLT Kbd", 0.6),
            ("AMP Atk", 0.12),
            ("AMP Rel", 0.4),
            ("CHORUS", 1.0),
            ("CHORUS ModRate", 0.2),
            ("CHORUS ModDepth", 0.35),
            ("CHORUS Mix", 0.4),
        ],
    ),
    p(
        "Lead",
        "Power Fifths",
        &[
            ("Voices Mode", 1.0),
            ("OSC Kind", 1.0),
            ("OSC Mod", 0.5),
            ("FLT Freq", 0.7),
            ("FLT Peak", 0.45),
            ("EGF Dec", 0.35),
            ("EGF Sus", 0.6),
            ("EGF Amnt", 0.6),
            ("AMP Rel", 0.3),
            ("DIST", 1.0),
            ("Overdrive", 0.4),
            ("DELAY", 1.0),
            ("DELAY Time", 0.7),
            ("DELAY Fdbk", 0.3),
            ("DELAY Mix", 0.35),
        ],
    ),
    p(
        "Pad",
        "Slow Tide",
        &[
            ("OSC Kind", 0.6),
            ("OSC Mod", 0.35),
            ("FLT Freq", 0.5),
            ("FLT Peak", 0.15),
            ("EGF Atk", 0.5),
            ("EGF Dec", 0.7),
            ("EGF Sus", 0.6),
            ("EGF Amnt", 0.62),
            ("LFO Rate", 0.7),
            ("LFO Amnt", 0.65),
            ("AMP Atk", 0.42),
            ("AMP Rel", 0.85),
            ("CHORUS", 1.0),
            ("CHORUS ModRate", 0.2),
            ("CHORUS ModDepth", 0.6),
            ("CHORUS Mix", 0.5),
            ("Post Gain", 0.325),
        ],
    ),
    p(
        "Pad",
        "Dust Halo",
        &[
            ("OSC Kind", 0.82),
            ("OSC Mod", 0.3),
            ("OSC Noise", 0.25),
            ("FLT Freq", 0.55),
            ("FLT Peak", 0.2),
            ("AMP Atk", 0.46),
            ("AMP Rel", 0.9),
            ("PHASER", 1.0),
            ("PHASER ModSpeed", 0.75),
            ("PHASER ModWidth", 0.5),
            ("PHASER Fdbk", 0.3),
            ("PHASER Mix", 0.6),
            ("Post Gain", 0.425),
        ],
    ),
    p(
        "Pad",
        "Octave Bloom",
        &[
            ("OSC Kind", 0.93),
            ("OSC Mod", 0.5),
            ("FLT Freq", 0.45),
            ("EGF Atk", 0.55),
            ("EGF Dec", 0.8),
            ("EGF Sus", 0.7),
            ("EGF Amnt", 0.66),
            ("AMP Atk", 0.38),
            ("AMP Rel", 0.8),
            ("DELAY", 1.0),
            ("DELAY Time", 1.0),
            ("DELAY Fdbk", 0.45),
            ("DELAY Filter", 0.4),
            ("DELAY Mix", 0.35),
            ("Post Gain", 0.275),
        ],
    ),
];

// ---- SuperSaw (23) ----
// `Osc. Unisono` N = int(11v + 1): 0.1 → 2, 0.25 → 3, 0.4 → 5, 0.5 → 6, 0.6 → 7, 0.75 → 9. `#1` is the SUPERSAW page's
// Pulse Width, `#12` the SUPERSYNC page's, `#53` the FX Gain. `LFO Dest` by round(7v): 0.29 Frequency, 0.571 Pulse
// Width. `Transpose` 0 is −12.

const SUPERSAW: &[Preset] = &[
    p(
        "Bass",
        "Stone Floor",
        &[
            ("Osc. Sub Level", 0.6),
            ("Filter Cutoff", 0.32),
            ("Filter Resonance", 0.3),
            ("Filter EG Decay", 0.35),
            ("Filter EG Sustain", 0.1),
            ("Filter EG Amount", 0.72),
            ("Level EG Release", 0.15),
            ("Transpose", 0.0),
            ("Monophonic Mode", 1.0),
            ("Distortion", 0.25),
            ("#53", 0.85),
        ],
    ),
    p(
        "Bass",
        "Wide Growl",
        &[
            ("#1", 0.6),
            ("Osc. Pulse Mode", 1.0),
            ("Osc. Unisono", 0.25),
            ("Osc. Unisono Detune", 0.4),
            ("Filter Cutoff", 0.38),
            ("Filter Resonance", 0.45),
            ("Filter EG Decay", 0.45),
            ("Filter EG Sustain", 0.3),
            ("Filter EG Amount", 0.68),
            ("Level", 0.85),
            ("Slide Held Mode", 1.0),
            ("Slide Time", 0.3),
            ("Monophonic Mode", 1.0),
        ],
    ),
    p(
        "Lead",
        "Skyline",
        &[
            ("Osc. Unisono", 0.6),
            ("Osc. Unisono Detune", 0.55),
            ("Osc. Unisono Pan", 0.6),
            ("Filter Cutoff", 0.7),
            ("Filter Resonance", 0.2),
            ("Filter EG Decay", 0.5),
            ("Filter EG Sustain", 0.6),
            ("Filter EG Amount", 0.6),
            ("Level EG Release", 0.35),
            ("Level", 0.85),
            ("Slide Time", 0.2),
            ("LFO Rate", 0.82),
            ("LFO Amount", 0.56),
            ("LFO Dest", 0.29),
            ("LFO Fade In", 0.5),
            ("Monophonic Mode", 1.0),
            ("Delay Time", 0.5),
            ("Delay Feedback", 0.45),
            ("Delay Mix", 0.45),
        ],
    ),
    p(
        "Lead",
        "Sync Sweep",
        &[
            ("SuperSync Mode", 1.0),
            ("Osc. Sync", 0.2),
            ("#12", 0.8),
            ("Osc. Sync Decay", 0.55),
            ("Osc. Sync Sustain", 0.2),
            ("Osc. Sync EG Amount", 0.85),
            ("Filter Cutoff", 0.75),
            ("Level", 0.8),
            ("Slide Time", 0.25),
            ("Monophonic Mode", 1.0),
            ("Distortion", 0.15),
        ],
    ),
    p(
        "Lead",
        "Square Pipe",
        &[
            ("#1", 1.0),
            ("Osc. Pulse Mode", 1.0),
            ("Osc. Sub Level", 0.3),
            ("Osc. Unisono", 0.1),
            ("Osc. Unisono Detune", 0.25),
            ("Filter Cutoff", 0.62),
            ("Filter KBD", 0.4),
            ("Level EG Attack", 0.15),
            ("Level", 0.75),
            ("Delay Time", 0.3),
            ("Delay Mix", 0.35),
            ("Monophonic Mode", 1.0),
            ("Post Gain", 0.425),
        ],
    ),
    p(
        "Pad",
        "Aurora Wash",
        &[
            ("Osc. Unisono", 0.75),
            ("Osc. Unisono Detune", 0.7),
            ("Osc. Unisono Pan", 1.0),
            ("Osc. Unisono Detune Curve", 0.6),
            ("Filter Cutoff", 0.48),
            ("Filter Resonance", 0.15),
            ("Filter EG Attack", 0.55),
            ("Filter EG Decay", 0.7),
            ("Filter EG Sustain", 0.7),
            ("Filter EG Amount", 0.62),
            ("Level EG Attack", 0.7),
            ("Level EG Release", 0.75),
            ("LFO Rate", 0.3),
            ("LFO Amount", 0.62),
            ("Number Of Voices", 1.0),
            ("Delay Mix", 0.3),
            ("Post Gain", 0.31),
        ],
    ),
    p(
        "Pad",
        "Warm Blanket",
        &[
            ("Osc. Sub Level", 0.4),
            ("Osc. Unisono", 0.4),
            ("Osc. Unisono Detune", 0.45),
            ("Filter Cutoff", 0.4),
            ("Filter Resonance", 0.1),
            ("Filter KBD", 0.3),
            ("Level EG Attack", 0.6),
            ("Level EG Release", 0.7),
            ("Number Of Voices", 1.0),
            ("Post Gain", 0.3),
        ],
    ),
    p(
        "Pad",
        "Pulse Choir",
        &[
            ("#1", 0.7),
            ("Osc. Pulse Mode", 1.0),
            ("Osc. Unisono", 0.5),
            ("Osc. Unisono Detune", 0.5),
            ("Osc. Unisono Pan", 0.8),
            ("Filter Cutoff", 0.55),
            ("Level EG Attack", 0.65),
            ("Level EG Release", 0.7),
            ("LFO Rate", 0.55),
            ("LFO Amount", 0.7),
            ("LFO Dest", 0.571),
            ("Level", 0.75),
            ("Number Of Voices", 1.0),
            ("Post Gain", 0.26),
        ],
    ),
    p(
        "Pluck",
        "Ice Pluck",
        &[
            ("Osc. Unisono", 0.25),
            ("Osc. Unisono Detune", 0.35),
            ("Filter Cutoff", 0.35),
            ("Filter Resonance", 0.35),
            ("Filter EG Decay", 0.3),
            ("Filter EG Sustain", 0.0),
            ("Filter EG Amount", 0.78),
            ("Level EG Decay", 0.42),
            ("Level EG Sustain", 0.0),
            ("Level EG Release", 0.3),
            ("Number Of Voices", 1.0),
            ("Delay Time", 0.3),
            ("Delay Feedback", 0.4),
            ("Delay Mix", 0.45),
            ("Post Gain", 0.45),
        ],
    ),
    p(
        "Pluck",
        "Warehouse Stab",
        &[
            ("Osc. Unisono", 0.6),
            ("Osc. Unisono Detune", 0.5),
            ("Filter Cutoff", 0.55),
            ("Filter EG Decay", 0.35),
            ("Filter EG Sustain", 0.0),
            ("Filter EG Amount", 0.7),
            ("Level EG Decay", 0.45),
            ("Level EG Sustain", 0.0),
            ("Level EG Release", 0.25),
            ("Level", 0.75),
            ("Number Of Voices", 1.0),
            ("Distortion", 0.2),
            ("Post Gain", 0.35),
        ],
    ),
    p(
        "Pluck",
        "Wood Mallet",
        &[
            ("#1", 1.0),
            ("Osc. Pulse Mode", 1.0),
            ("Osc. Sub Level", 0.5),
            ("Filter Cutoff", 0.42),
            ("Filter KBD", 0.5),
            ("Filter VEL", 0.3),
            ("Filter EG Decay", 0.35),
            ("Filter EG Sustain", 0.0),
            ("Filter EG Amount", 0.7),
            ("Level EG Decay", 0.5),
            ("Level EG Sustain", 0.0),
            ("Level EG Release", 0.4),
            ("Number Of Voices", 1.0),
            ("Post Gain", 0.2),
        ],
    ),
];

// ---- Transistor Bass (19) ----
// `#7` the DISTORTION toggle, `#8` Drive HP, `#9` Drive, `#10` Drive Vol., `#11` PW, `#12` LFO Speed, `#13` LFO->PW.

const TRANSISTOR_BASS: &[Preset] = &[
    p(
        "",
        "Acid Drip",
        &[
            ("Waveform", 1.0),
            ("Cutoff", 0.55),
            ("Reso", 0.85),
            ("Env Mod", 0.7),
            ("Decay", 0.45),
            ("Accent", 0.7),
            ("#7", 1.0),
            ("#9", 0.0),
            ("#10", 0.6),
        ],
    ),
    p(
        "",
        "Rubber Square",
        &[
            ("Waveform", 0.0),
            ("Cutoff", 0.6),
            ("Reso", 0.5),
            ("Env Mod", 0.45),
            ("Decay", 0.3),
            ("Accent", 0.4),
            ("#7", 1.0),
            ("#9", 0.0),
            ("#10", 0.79),
        ],
    ),
    p(
        "",
        "Night Squelch",
        &[
            ("Waveform", 1.0),
            ("Cutoff", 0.5),
            ("Reso", 1.0),
            ("Env Mod", 0.9),
            ("Decay", 0.6),
            ("Accent", 0.9),
            ("#7", 1.0),
            ("#8", 0.1),
            ("#9", 0.2),
            ("#10", 0.85),
        ],
    ),
    p(
        "",
        "Low Hum",
        &[
            ("Waveform", 0.3),
            ("Cutoff", 0.7),
            ("Reso", 0.3),
            ("Env Mod", 0.15),
            ("Decay", 1.0),
            ("Accent", 0.1),
            ("#7", 1.0),
            ("#9", 0.0),
            ("#10", 1.0),
        ],
    ),
    p(
        "",
        "Fuzz Line",
        &[
            ("Waveform", 0.8),
            ("Cutoff", 0.65),
            ("Reso", 0.6),
            ("Env Mod", 0.5),
            ("Decay", 0.5),
            ("Accent", 0.5),
            ("#7", 1.0),
            ("#8", 0.25),
            ("#9", 0.25),
            ("#10", 1.0),
        ],
    ),
    p(
        "",
        "Thin Wire",
        &[
            ("Waveform", 0.0),
            ("Cutoff", 0.78),
            ("Reso", 0.55),
            ("Env Mod", 0.5),
            ("Decay", 0.15),
            ("Accent", 0.3),
            ("#11", 0.3),
            ("#12", 0.1),
            ("#13", 0.7),
            ("#7", 1.0),
            ("#9", 0.0),
            ("#10", 0.68),
        ],
    ),
    p(
        "",
        "Bubble Bath",
        &[
            ("Waveform", 0.5),
            ("Cutoff", 0.58),
            ("Reso", 0.92),
            ("Env Mod", 0.8),
            ("Decay", 0.15),
            ("Accent", 0.6),
            ("#7", 1.0),
            ("#9", 0.0),
            ("#10", 0.58),
        ],
    ),
    p(
        "",
        "Warm Tape",
        &[
            ("Waveform", 1.0),
            ("Cutoff", 0.75),
            ("Reso", 0.27),
            ("Env Mod", 0.2),
            ("Decay", 0.8),
            ("Accent", 0.2),
            ("#7", 1.0),
            ("#9", 0.11),
            ("#10", 0.85),
        ],
    ),
];

#[cfg(test)]
mod tests;
