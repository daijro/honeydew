//! Our GMSynth bank: 14 presets in Bass, Bells, Keys, Lead and Pad, our own sound design and names.
//! OSC A and B are `CSTM` settings, not params: each preset sets
//! both shapes by menu slot through [`super::Preset::settings`].
//!
//! Laws, with menu values mid-step: `Osc B Pitch` trunc(48v − 24) st (+12
//! 0.76042, −12 0.23958, +17 0.8646, +24 1); `Osc Mod Kind` int(3v) (FM 0.5, Sync 0.8333); `Filter Kind` int(2v) (LP 24
//! 0.75); `LFO Rate` and `FX n Sync` int(9v) (k + 0.5)/9; `LFO Dest` int(4v) (FLT 0.125, FREQ 0.625, LVL 0.875); `FX n
//! Kind` int(9v) (PHASER 0.2778, REVERB 0.7222; FX 2 is DELAY by default); `Choir Num Voices` int(3v) + 1 (2 voices
//! 0.5, 3 0.8333, 4 1). `Osc B Mix` above 0.5 adds B and takes from A. Chord presets trim with `Post Gain`.

use super::Preset;

/// The OSC `A` and `B` shapes' `CSTM` settings keys (`crate::panels::cstm::GMSYNTH`).
const A: u32 = 21100;
const B: u32 = 21200;

/// A GMSynth preset on the OSC shapes `shapes` (`[(A, slot), (B, slot)]`, slots 0–28 of the menu).
const fn gm(
    category: &'static str,
    name: &'static str,
    shapes: &'static [(u32, f32)],
    params: &'static [(&'static str, f32)],
) -> Preset {
    Preset {
        category,
        name,
        params,
        settings: shapes,
    }
}

pub(super) const BANK: &[Preset] = &[
    // Smooth Sub with Square Retro an octave up, mono with auto glide: a round sub.
    gm(
        "Bass",
        "Iron Sub",
        &[(A, 19.0), (B, 20.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc B Mix", 0.62),
            ("Filter Kind", 0.75),
            ("Cutoff", 0.42),
            ("Resonance", 0.15),
            ("Filter Decay", 0.45),
            ("Filter Contour", 0.82),
            ("Single Voice", 1.0),
            ("Slide Time", 0.25),
            ("Auto Portamento", 1.0),
            ("Level Release", 0.3),
        ],
    ),
    // Saw frequency-modulated by a Sine an octave up, the index decaying: a growl that settles.
    gm(
        "Bass",
        "Snarl Bass",
        &[(A, 16.0), (B, 18.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc Mod Kind", 0.5),
            ("OSc Mod", 0.15),
            ("Osc Mod Decay", 0.5),
            ("Osc Mod Contour", 0.85),
            ("Cutoff", 0.55),
            ("Resonance", 0.2),
            ("Filter Decay", 0.4),
            ("Filter Contour", 0.8),
            ("Single Voice", 1.0),
            ("Level Release", 0.25),
            ("Post Gain", 0.45),
        ],
    ),
    // Funky with Square an octave down, a resonant filter snap.
    gm(
        "Bass",
        "Wah Pluck",
        &[(A, 8.0), (B, 22.0)],
        &[
            ("Osc B Pitch", 0.23958),
            ("Osc B Mix", 0.6),
            ("Cutoff", 0.32),
            ("Resonance", 0.55),
            ("Filter Decay", 0.42),
            ("Filter Contour", 0.88),
            ("Level Decay", 0.55),
            ("Level Sustain", 0.45),
            ("Level Release", 0.2),
            ("Single Voice", 1.0),
            ("Post Gain", 0.425),
        ],
    ),
    // Sine frequency-modulated by a Sine 17 st (and a touch) up, the index decaying, through the reverb: a glassy bell.
    gm(
        "Bells",
        "Frost Chime",
        &[(A, 18.0), (B, 18.0)],
        &[
            ("Osc B Pitch", 0.8646),
            ("Osc B Detune", 0.62),
            ("Osc Mod Kind", 0.5),
            ("OSc Mod", 0.1),
            ("Osc Mod Decay", 0.6),
            ("Osc Mod Contour", 0.9),
            ("Level Decay", 0.66),
            ("Level Sustain", 0.0),
            ("Level Release", 0.9),
            ("FX 3 Kind", 0.7222),
            ("FX 3 Enable", 1.0),
            ("Post Gain", 0.48),
            ("FX 3 Param X", 0.7),
        ],
    ),
    // Triangle with a Sine two octaves up, short, through the delay.
    gm(
        "Bells",
        "Tin Carousel",
        &[(A, 26.0), (B, 18.0)],
        &[
            ("Osc B Pitch", 1.0),
            ("Osc B Mix", 0.64),
            ("Level Decay", 0.56),
            ("Level Sustain", 0.0),
            ("Level Release", 0.6),
            ("FX 2 Enable", 1.0),
            ("FX 2 Sync", 0.2778),
            ("FX 2 Param X", 0.55),
            ("Post Gain", 0.4),
        ],
    ),
    // Rhodes with a Sine an octave up and a tremolo (LFO on the level): an electric piano.
    gm(
        "Keys",
        "Warm Tremolo",
        &[(A, 13.0), (B, 18.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc B Mix", 0.58),
            ("Level Decay", 0.66),
            ("Level Sustain", 0.35),
            ("Level Release", 0.5),
            ("LFO Rate", 0.0556),
            ("LFO Dest", 0.875),
            ("LFO Ampl.", 0.62),
            ("Post Gain", 0.275),
        ],
    ),
    // Keys with Square Smooth an octave up, two choir voices through the phaser: an organ.
    gm(
        "Keys",
        "Drawbar Haze",
        &[(A, 10.0), (B, 21.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc B Mix", 0.66),
            ("Level Attack", 0.12),
            ("Level Release", 0.35),
            ("Choir Num Voices", 0.5),
            ("Choir Detune", 0.15),
            ("Choir Stereo", 0.5),
            ("FX 1 Kind", 0.2778),
            ("FX 1 Enable", 1.0),
            ("Post Gain", 0.3),
        ],
    ),
    // Electro and Square Retro on a key-tracked filter snap.
    gm(
        "Keys",
        "Funk Clav",
        &[(A, 5.0), (B, 20.0)],
        &[
            ("Osc B Mix", 0.6),
            ("Osc B Detune", 0.55),
            ("Filter Kbd. Scl.", 1.0),
            ("Cutoff", 0.5),
            ("Resonance", 0.35),
            ("Filter Decay", 0.4),
            ("Filter Contour", 0.82),
            ("Level Decay", 0.52),
            ("Level Sustain", 0.2),
            ("Level Release", 0.25),
            ("Post Gain", 0.375),
        ],
    ),
    // Saw Mean with a detuned Saw, LP 24, mono glide through the delay.
    gm(
        "Lead",
        "Laser Line",
        &[(A, 14.0), (B, 16.0)],
        &[
            ("Osc B Detune", 0.58),
            ("Osc B Mix", 0.7),
            ("Filter Kind", 0.75),
            ("Cutoff", 0.66),
            ("Resonance", 0.3),
            ("Single Voice", 1.0),
            ("Slide Time", 0.3),
            ("Auto Portamento", 1.0),
            ("Level Release", 0.3),
            ("FX 2 Enable", 1.0),
            ("FX 2 Sync", 0.3889),
            ("FX 2 Param X", 0.5),
        ],
    ),
    // Saw synced to a Square, the sync sweep on the mod envelope.
    gm(
        "Lead",
        "Sync Scream",
        &[(A, 16.0), (B, 22.0)],
        &[
            ("Osc Mod Kind", 0.8333),
            ("OSc Mod", 0.2),
            ("Osc Mod Decay", 0.58),
            ("Osc Mod Contour", 0.95),
            ("Cutoff", 0.72),
            ("Resonance", 0.2),
            ("Single Voice", 1.0),
            ("Slide Time", 0.2),
            ("Level Release", 0.3),
        ],
    ),
    // Vocal with Throat an octave up and a vibrato, a soft attack.
    gm(
        "Lead",
        "Vowel Whistle",
        &[(A, 28.0), (B, 25.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc B Mix", 0.62),
            ("LFO Rate", 0.1667),
            ("LFO Dest", 0.625),
            ("LFO Ampl.", 0.58),
            ("Cutoff", 0.7),
            ("Single Voice", 1.0),
            ("Slide Time", 0.25),
            ("Level Attack", 0.25),
            ("Level Release", 0.35),
            ("Post Gain", 0.425),
        ],
    ),
    // Silk with Saw an octave up, four choir voices spread, through a soft echo and the reverb (the release is at its
    // longest, 0.11 s by law; the echo carries the tail).
    gm(
        "Pad",
        "Silk Drift",
        &[(A, 17.0), (B, 16.0)],
        &[
            ("Osc B Pitch", 0.76042),
            ("Osc B Mix", 0.6),
            ("Choir Num Voices", 1.0),
            ("Choir Detune", 0.12),
            ("Choir Stereo", 0.8),
            ("Cutoff", 0.58),
            ("Level Attack", 0.62),
            ("Level Release", 1.0),
            ("FX 2 Enable", 1.0),
            ("FX 2 Sync", 0.3889),
            ("FX 2 Param X", 0.35),
            ("FX 3 Kind", 0.7222),
            ("FX 3 Enable", 1.0),
            ("FX 3 Param X", 0.7),
            ("Post Gain", 0.24),
        ],
    ),
    // Vocal and String with breath noise, three voices, a slow filter LFO and the delay.
    gm(
        "Pad",
        "Breath Choir",
        &[(A, 28.0), (B, 24.0)],
        &[
            ("Osc B Mix", 0.6),
            ("Osc B Detune", 0.56),
            ("Osc Noise", 0.2),
            ("Choir Num Voices", 0.8333),
            ("Choir Detune", 0.3),
            ("Choir Stereo", 0.7),
            ("LFO Rate", 0.6111),
            ("LFO Dest", 0.125),
            ("LFO Ampl.", 0.68),
            ("Cutoff", 0.62),
            ("Level Attack", 0.66),
            ("Level Release", 1.0),
            ("FX 2 Enable", 1.0),
            ("FX 2 Sync", 0.3889),
            ("FX 2 Param X", 0.6),
            ("Post Gain", 0.26),
        ],
    ),
    // String with Saw an octave down, the filter opening slowly, octave choir and the reverb.
    gm(
        "Pad",
        "Low Cathedral",
        &[(A, 24.0), (B, 16.0)],
        &[
            ("Osc B Pitch", 0.23958),
            ("Osc B Mix", 0.62),
            ("Filter Kind", 0.75),
            ("Cutoff", 0.4),
            ("Filter Attack", 0.7),
            ("Filter Decay", 1.0),
            ("Filter Contour", 0.86),
            ("Level Attack", 0.58),
            ("Level Release", 1.0),
            ("Choir Num Voices", 0.5),
            ("Choir Scale", 1.0),
            ("Choir Detune", 0.2),
            ("Choir Stereo", 0.6),
            ("FX 3 Kind", 0.7222),
            ("FX 3 Enable", 1.0),
            ("Post Gain", 0.25),
            ("FX 3 Param X", 0.7),
        ],
    ),
];
