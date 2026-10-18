//! General MIDI and the sounds Honeydew gives imported channels: MIDI import ([`super::import`]) and FLP import
//! (`crate::flp`) share these.
//!
//! Owner request (2026-10-18): every imported channel makes sound. A GM program loads our closest bundled DW program
//! ([`BUNDLED`]), or without the bundle (a build without `content/`) our code-made preset for its family ([`CODE`]);
//! our programs play at concert pitch. MIDI channel 10 plays on a Drums channel with our kit ([`drum_kit`]), each GM
//! drum key on the pad that fits it best ([`drum_pad`]).

use std::path::{Path, PathBuf};

use crate::content::{Factory, ROOT, content_path};
use crate::panels::cstm::pick_dw_program;
use crate::song::{ChannelKind, DrumKit, ModuleInstance, Rack, Song, SynthKind};
use crate::ui::main_menu::SongLibrary;

/// The General MIDI program names, 0-based, a family of 8 a line.
#[rustfmt::skip]
pub const PROGRAMS: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano", "Electric Piano 1",
        "Electric Piano 2", "Harpsichord", "Clavinet",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica",
        "Tango Accordion",
    "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)", "Electric Guitar (clean)",
        "Electric Guitar (muted)", "Overdriven Guitar", "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)", "Fretless Bass", "Slap Bass 1", "Slap Bass 2",
        "Synth Bass 1", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2", "Choir Aahs", "Voice Oohs",
        "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet", "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Lead 1 (square)", "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)", "Lead 5 (charang)",
        "Lead 6 (voice)", "Lead 7 (fifths)", "Lead 8 (bass + lead)",
    "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)", "Pad 4 (choir)", "Pad 5 (bowed)", "Pad 6 (metallic)",
        "Pad 7 (halo)", "Pad 8 (sweep)",
    "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)", "FX 5 (brightness)", "FX 6 (goblins)",
        "FX 7 (echoes)", "FX 8 (sci-fi)",
    "Sitar", "Banjo", "Shamisen", "Koto", "Kalimba", "Bagpipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock", "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet", "Telephone Ring", "Helicopter", "Applause",
        "Gunshot",
];

const PIANO: &str = "Piano/Grand Piano";
const GLOCK: &str = "Mallets/Glockenspiel";
const MARIMBA: &str = "Mallets/Marimba";
const REED: &str = "Lead/Reed Lead";
const NYLON: &str = "Guitar/Nylon Guitar";
const STEEL: &str = "Guitar/Steel Guitar";
const ELECTRIC: &str = "Guitar/Clean Electric";
const DRIVEN: &str = "Guitar/Shinyguitar Pickup";
const FINGER: &str = "Bass/Finger Bass";
const SYNTH_BASS: &str = "Synth Bass/Synth Bass";
const VIOLIN: &str = "Orchestral/Solo Violin";
const HARP: &str = "Orchestral/Concert Harp";
const SECTION: &str = "Strings/String Section";
const CHOIR: &str = "Choir/Choir Aahs";
const OOHS: &str = "Choir/Voice Oohs";
const TRUMPET: &str = "Brass/Trumpet";
const LOW_BRASS: &str = "Brass/Low Brass";
const CLARINET: &str = "Woodwinds/Clarinet";
const FLUTE: &str = "Woodwinds/Flute";
const SQUARE: &str = "Lead/Square Lead";
const SHIMMER: &str = "Pad/Shimmer Pad";
const SWEEP: &str = "Pad/Sweep Pad";
const TIMPANI: &str = "Orchestral/Timpani";
const ORGAN: &str = "Keyboard/Chapel Organ";

/// Our bundled DW program for each GM program, below `Presets/DW Sampler/`, without `.dwp`; a family a line.
#[rustfmt::skip]
const BUNDLED: [&str; 128] = [
    PIANO, "Piano/Bright Grand", "Keyboard/Electric Grand", "Piano/Honky-Tonk", "Keyboard/Electric Piano",
        "Keyboard/FM Tines", "Keyboard/Harpsichord", "Keyboard/Clavi",
    GLOCK, GLOCK, GLOCK, MARIMBA, MARIMBA, MARIMBA, GLOCK, HARP,
    "Keyboard/Drawbar Organ", "Keyboard/Percussive Organ", "Keyboard/Rock Organ", ORGAN, ORGAN, REED, REED, REED,
    NYLON, STEEL, ELECTRIC, ELECTRIC, "Guitar/Green Guitar Staccato", DRIVEN, DRIVEN, ELECTRIC,
    "Bass/Upright Bass", FINGER, FINGER, FINGER, FINGER, FINGER, SYNTH_BASS, SYNTH_BASS,
    VIOLIN, "Orchestral/Viola Section", "Strings/Cello", "Orchestral/Double Bass", "Orchestral/Tremolo Strings",
        "Strings/Pizzicato", HARP, TIMPANI,
    SECTION, SECTION, SECTION, SECTION, CHOIR, OOHS, OOHS, SECTION,
    TRUMPET, LOW_BRASS, LOW_BRASS, TRUMPET, "Brass/French Horn", TRUMPET, LOW_BRASS, LOW_BRASS,
    REED, REED, REED, REED, CLARINET, CLARINET, CLARINET, CLARINET,
    FLUTE, FLUTE, FLUTE, FLUTE, FLUTE, FLUTE, FLUTE, FLUTE,
    SQUARE, SQUARE, SQUARE, SQUARE, SQUARE, OOHS, SQUARE, SYNTH_BASS,
    SHIMMER, SWEEP, SWEEP, CHOIR, SWEEP, SHIMMER, SHIMMER, SWEEP,
    SHIMMER, SWEEP, SHIMMER, SWEEP, SHIMMER, SWEEP, SHIMMER, SWEEP,
    STEEL, STEEL, STEEL, NYLON, MARIMBA, REED, VIOLIN, REED,
    GLOCK, MARIMBA, MARIMBA, MARIMBA, TIMPANI, TIMPANI, TIMPANI, SHIMMER,
    STEEL, SWEEP, SWEEP, SHIMMER, GLOCK, SWEEP, SWEEP, TIMPANI,
];

/// Our code-made preset for each GM family of 8 programs, `(module type, name)`: what plays without the bundle.
const CODE: [(SynthKind, &str); 16] = [
    (SynthKind::MiniSynth, "Felt Keys"),
    (SynthKind::GmSynth, "Frost Chime"),
    (SynthKind::MiniSynth, "Reed Organ"),
    (SynthKind::SuperSaw, "Ice Pluck"),
    (SynthKind::MiniSynth, "Velvet Sub"),
    (SynthKind::SuperSaw, "Warm Blanket"),
    (SynthKind::SuperSaw, "Warm Blanket"),
    (SynthKind::ThreeXOsc, "Brass Swell"),
    (SynthKind::MiniSynth, "Hollow Reed"),
    (SynthKind::GmSynth, "Vowel Whistle"),
    (SynthKind::MiniSynth, "Neon Ribbon"),
    (SynthKind::MiniSynth, "Slow Tide"),
    (SynthKind::SuperSaw, "Aurora Wash"),
    (SynthKind::SuperSaw, "Ice Pluck"),
    (SynthKind::SuperSaw, "Wood Mallet"),
    (SynthKind::SuperSaw, "Aurora Wash"),
];

/// The GM program name of `program` (0-based; 7 bits).
pub fn program_name(program: u8) -> &'static str {
    PROGRAMS[usize::from(program & 0x7F)]
}

/// The factory path of our bundled DW program for GM `program`.
fn bundled(program: u8) -> PathBuf {
    let rel = BUNDLED[usize::from(program & 0x7F)];
    Path::new(ROOT).join(format!("Presets/DW Sampler/{rel}.dwp"))
}

/// Gives channel `index` the sound for GM `program`: our bundled DW program when `factory` has it, in a DW Sampler,
/// else our code-made preset for the program's family in its own module (the channel's kind follows). It takes the
/// place of the rack's instrument; the effects stay. Returns the name of what it loaded.
pub fn load_program(song: &mut Song, index: usize, program: u8, factory: &Factory) -> String {
    let channel = &mut song.channels[index];
    let path = bundled(program);
    if let Some(shorthand) = content_path(&path).filter(|_| factory.exists(&path)) {
        channel.kind = ChannelKind::Synth(SynthKind::DwSampler);
        let module = instrument(&mut channel.rack, SynthKind::DwSampler);
        pick_dw_program(module, &shorthand, None);
        named(module, &path);
        return module.file.preset_name.clone();
    }
    let (kind, name) = CODE[usize::from(program & 0x7F) / 8];
    channel.kind = ChannelKind::Synth(kind);
    let module = instrument(&mut channel.rack, kind);
    if let Some(preset) = crate::content::preset(kind.type_id(), name) {
        module.load_preset(preset);
    }
    name.to_owned()
}

/// A new `kind` module at the top of `rack`, in place of its instruments.
pub fn instrument(rack: &mut Rack, kind: SynthKind) -> &mut ModuleInstance {
    let instruments = |m: &ModuleInstance| {
        crate::params::module_spec(m.type_id)
            .is_none_or(|s| s.role == crate::params::Role::Instrument)
    };
    rack.modules.retain(|m| !instruments(m));
    let id = rack.insert(0, kind.type_id());
    rack.get_mut(id).expect("just added")
}

/// What an imported channel with no sound of its own plays, guessed from its names ([`guess`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandIn {
    /// A GM program ([`load_program`]).
    Program(u8),
    /// One drum sound: every note hits our kit's pad of that name.
    Pad(&'static str),
    /// A drum kit played by GM keys, as MIDI channel 10 is.
    Kit,
}

/// Name words and the stand-in they ask for, checked in order against the words of a name (lower case, split at
/// anything that is no letter or digit).
const WORDS: &[(&[&str], StandIn)] = &[
    (&["kick", "kik", "bd", "bassdrum"], StandIn::Pad("Kick")),
    (&["snare", "snr"], StandIn::Pad("Snare")),
    (&["clap", "clp", "handclap"], StandIn::Pad("Clap")),
    (&["rim", "rimshot", "sidestick"], StandIn::Pad("Rim")),
    (&["openhat", "ohat"], StandIn::Pad("Open Hat")),
    (
        &["hat", "hats", "hihat", "hh", "chh"],
        StandIn::Pad("Closed Hat"),
    ),
    (&["crash", "cymbal", "cym", "splash"], StandIn::Pad("Crash")),
    (&["ride"], StandIn::Pad("Ride")),
    (&["tom", "toms"], StandIn::Pad("Mid Tom")),
    (&["cowbell"], StandIn::Pad("Cowbell")),
    (
        &["shaker", "tamb", "tambourine", "perc", "percussion"],
        StandIn::Pad("Shaker"),
    ),
    (
        &[
            "fpc",
            "drum",
            "drums",
            "kit",
            "drumkit",
            "battery",
            "beat",
            "breakbeat",
            "drumaxx",
            "slicex",
        ],
        StandIn::Kit,
    ),
    (&["808", "sub", "reese", "subbass"], StandIn::Program(38)),
    (&["bass", "boobass", "bassline"], StandIn::Program(33)),
    (
        &[
            "rhodes",
            "epiano",
            "ep",
            "wurli",
            "wurlitzer",
            "keys",
            "dx10",
        ],
        StandIn::Program(4),
    ),
    (&["piano", "grand", "keyscape"], StandIn::Program(0)),
    (&["clav", "clavinet"], StandIn::Program(7)),
    (&["organ"], StandIn::Program(16)),
    (
        &["bell", "bells", "glock", "celesta", "chime"],
        StandIn::Program(9),
    ),
    (
        &["marimba", "mallet", "vibes", "xylo", "kalimba"],
        StandIn::Program(12),
    ),
    (&["guitar", "gtr", "nylon"], StandIn::Program(24)),
    (&["violin", "fiddle"], StandIn::Program(40)),
    (&["cello"], StandIn::Program(42)),
    (&["harp"], StandIn::Program(46)),
    (
        &["pizz", "pizzicato", "pluck", "plucks", "plucked"],
        StandIn::Program(45),
    ),
    (
        &[
            "string",
            "strings",
            "orchestra",
            "orch",
            "ensemble",
            "sakura",
        ],
        StandIn::Program(48),
    ),
    (
        &["choir", "vox", "vocal", "vocals", "voice", "aah", "ooh"],
        StandIn::Program(53),
    ),
    (&["trumpet"], StandIn::Program(56)),
    (
        &["brass", "horn", "horns", "trombone"],
        StandIn::Program(61),
    ),
    (&["sax", "saxophone"], StandIn::Program(65)),
    (
        &["flute", "woodwind", "clarinet", "oboe"],
        StandIn::Program(73),
    ),
    (
        &[
            "pad", "pads", "atmos", "ambient", "texture", "drone", "ogun",
        ],
        StandIn::Program(89),
    ),
    (
        &[
            "fx",
            "riser",
            "sweep",
            "impact",
            "noise",
            "uplifter",
            "downlifter",
        ],
        StandIn::Program(95),
    ),
    (
        &[
            "lead",
            "leads",
            "saw",
            "square",
            "supersaw",
            "synth",
            "arp",
            "serum",
            "sylenth",
            "sylenth1",
            "massive",
            "vital",
            "spire",
            "harmor",
            "sytrus",
            "harmless",
            "poizone",
            "toxic",
            "gms",
            "minisynth",
            "morphine",
        ],
        StandIn::Program(81),
    ),
];

/// The stand-in `names` ask for, the first name first (a channel's own name, then its plugin's): by the first word
/// any of them has in [`WORDS`]; else a neutral piano.
pub fn guess(names: &[&str]) -> StandIn {
    let words = |name: &str| {
        let lower = name.to_lowercase();
        (lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty()))
        .map(str::to_owned)
        .collect::<Vec<_>>()
    };
    for name in names {
        let words = words(name);
        // `open hat` and `open hi hat` are one sound.
        if words.iter().any(|w| w == "open") && words.iter().any(|w| w.contains("hat")) {
            return StandIn::Pad("Open Hat");
        }
        let found = WORDS
            .iter()
            .find(|(list, _)| words.iter().any(|w| list.contains(&w.as_str())));
        if let Some(&(_, stand_in)) = found {
            return stand_in;
        }
    }
    StandIn::Program(0)
}

/// Gives channel `index` the stand-in `names` ask for ([`guess`]): a program, or a Drums channel on our kit whose
/// notes become hits (on one pad, or by GM key). Returns the name of what it plays.
pub fn stand_in(song: &mut Song, index: usize, names: &[&str], factory: &Factory) -> String {
    match guess(names) {
        StandIn::Program(program) => load_program(song, index, program, factory),
        StandIn::Pad(pad) => {
            let Some((kit, kit_name)) = drum_kit(factory) else {
                return load_program(song, index, 0, factory);
            };
            let names = drum_sound(gm_key(pad).unwrap_or(36));
            let at = (names.iter().chain([&pad]))
                .find_map(|n| {
                    kit.slots
                        .iter()
                        .position(|s| s.pad_name().eq_ignore_ascii_case(n))
                })
                .unwrap_or(0);
            let shown = kit.slots.get(at).map_or(pad, |s| s.pad_name()).to_owned();
            let at = u8::try_from(at).unwrap_or(0);
            to_drums(song, index, kit, |_| at);
            format!("{kit_name} {shown}")
        }
        StandIn::Kit => match drum_kit(factory) {
            Some((kit, kit_name)) => {
                let pads = kit.clone();
                to_drums(song, index, kit, |key| drum_pad(&pads, key));
                kit_name.to_owned()
            }
            None => load_program(song, index, 0, factory),
        },
    }
}

/// Makes channel `index` a Drums channel on `kit`: its instruments go (its effects stay), and its notes lanes become
/// notes-grid drum lanes whose notes hit the pad `pad(key)`, keeping their times, lengths, velocities and pans.
pub fn to_drums(song: &mut Song, index: usize, kit: DrumKit, pad: impl Fn(u8) -> u8) {
    let channel = &mut song.channels[index];
    channel.kind = ChannelKind::Drums;
    channel.drum_kit = Some(kit);
    let instruments = |m: &ModuleInstance| {
        crate::params::module_spec(m.type_id)
            .is_none_or(|s| s.role == crate::params::Role::Instrument)
    };
    channel.rack.modules.retain(|m| !instruments(m));
    for lane in channel
        .lanes
        .iter_mut()
        .filter(|l| l.kind == crate::song::LaneKind::Notes)
    {
        lane.kind = crate::song::LaneKind::Drums;
        lane.grid = crate::song::DrumGrid::Notes;
        for clip in &mut lane.clips {
            for event in &mut clip.events {
                if let crate::song::Event::Note {
                    pos,
                    len_beats,
                    key,
                    velocity,
                    pan,
                    ..
                } = *event
                {
                    *event = crate::song::Event::Drum {
                        pos,
                        len_beats,
                        pad: pad(key),
                        velocity,
                        pan,
                        value: crate::song::Event::DEFAULT_PITCH,
                        repeat: 0,
                    };
                }
            }
        }
    }
}

/// `module` named as a pick of the factory file `path` names it.
fn named(module: &mut ModuleInstance, path: &Path) {
    module.name_preset(path, Path::new(ROOT));
    module.file.source_path = module.file.preset_path.clone();
}

/// Our bundled acoustic kit, when the bundle is there.
const ROCK_KIT: &str = "Drumsets/Rock Kit.tmpl";
/// Our code-made kit with a pad for most GM drum sounds, which plays without the bundle.
const RELAY_KIT: &str = "Drumsets/Relay Kit.tmpl";

/// The kit GM drums play on, and its name: our bundled Rock Kit when `factory` has it, else the code-made Relay Kit
/// (`None` only for a factory with neither, a test's). Each slot's MIDI note is the GM key it stands for, so a GM
/// keyboard plays it and MIDI export writes the keys back ([`gm_key`]).
pub fn drum_kit(factory: &Factory) -> Option<(DrumKit, &'static str)> {
    let (file, name) = [(ROCK_KIT, "Rock Kit"), (RELAY_KIT, "Relay Kit")]
        .into_iter()
        .find_map(|(rel, name)| {
            let bytes = factory.read(&Path::new(ROOT).join(rel)).ok()?;
            Some((crate::flm::read_drumset(&bytes).ok()?, name))
        })?;
    let mut kit = DrumKit {
        file: crate::song::KitFileState {
            pads: file.file.pads,
            ..Default::default()
        },
        ..DrumKit::default()
    };
    for mut slot in file.slots {
        (slot.file.output_bus, slot.file.send_target) = (-1, -1);
        if let Some(key) = gm_key(slot.pad_name()) {
            slot.file.midi_key = u16::from(key);
        }
        kit.insert_slot(kit.slots.len(), slot);
    }
    Some((kit, name))
}

/// Pad names by GM drum sound, best first: the first one a kit has plays the key ([`drum_pad`]).
const KICK: &[&str] = &["Kick", "Hard Kick"];
const SNARE: &[&str] = &["Snare", "Rimshot", "Clap"];
const RIM: &[&str] = &["Side Stick", "Rim", "Rimshot", "Snare"];
const CLAP: &[&str] = &["Clap", "Snare"];
const CLOSED: &[&str] = &["Closed Hat", "Pedal Hat", "Shaker"];
const PEDAL: &[&str] = &["Pedal Hat", "Closed Hat"];
const OPEN: &[&str] = &["Open Hat", "Closed Hat"];
const LOW_TOM: &[&str] = &["Low Tom", "Tom", "Mid Tom"];
const MID_TOM: &[&str] = &["Mid Tom", "Tom", "Low Tom"];
const HIGH_TOM: &[&str] = &["High Tom", "Tom", "Mid Tom"];
const CRASH: &[&str] = &["Crash", "Cymbal", "Splash", "China", "Ride"];
const RIDE: &[&str] = &["Ride", "Cymbal", "Crash"];
const BELL: &[&str] = &["Ride Bell", "Cowbell", "Ride"];
const COWBELL: &[&str] = &["Cowbell", "Ride Bell", "Rim", "Side Stick"];
const SHAKER: &[&str] = &["Shaker", "Closed Hat"];
const PERC: &[&str] = &["Perc", "Rim", "Side Stick", "Cowbell", "Low Tom", "Tom"];

/// The sound of GM drum key `key` (MIDI channel 10).
fn drum_sound(key: u8) -> &'static [&'static str] {
    match key {
        ..=36 => KICK,
        37 => RIM,
        38 | 40 => SNARE,
        39 => CLAP,
        42 => CLOSED,
        44 => PEDAL,
        46 => OPEN,
        41 | 43 => LOW_TOM,
        45 | 47 => MID_TOM,
        48 | 50 => HIGH_TOM,
        49 | 52 | 55 | 57 => CRASH,
        51 | 59 => RIDE,
        53 => BELL,
        56 => COWBELL,
        54 | 69 | 70 | 82 => SHAKER,
        _ => PERC,
    }
}

/// The pad of `kit` that GM drum key `key` plays: the first pad named as [`drum_sound`] lists, else pad 0.
pub fn drum_pad(kit: &DrumKit, key: u8) -> u8 {
    let names = drum_sound(key);
    let pad = names
        .iter()
        .find_map(|name| (kit.slots.iter()).position(|s| s.pad_name().eq_ignore_ascii_case(name)));
    pad.and_then(|p| u8::try_from(p).ok()).unwrap_or(0)
}

/// The GM drum key a pad named `pad` stands for, `None` for a name GM has no key for.
pub fn gm_key(pad: &str) -> Option<u8> {
    const KEYS: [(&str, u8); 20] = [
        ("Kick", 36),
        ("Hard Kick", 35),
        ("Snare", 38),
        ("Rimshot", 40),
        ("Side Stick", 37),
        ("Rim", 37),
        ("Clap", 39),
        ("Closed Hat", 42),
        ("Pedal Hat", 44),
        ("Open Hat", 46),
        ("Low Tom", 41),
        ("Tom", 45),
        ("Mid Tom", 47),
        ("High Tom", 50),
        ("Crash", 49),
        ("Cymbal", 49),
        ("Splash", 55),
        ("China", 52),
        ("Ride", 51),
        ("Ride Bell", 53),
    ];
    const MORE: [(&str, u8); 3] = [("Cowbell", 56), ("Shaker", 82), ("Perc", 76)];
    (KEYS.iter().chain(&MORE))
        .find(|(name, _)| name.eq_ignore_ascii_case(pad))
        .map(|&(_, key)| key)
}
