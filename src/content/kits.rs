//! Our drum kits, `Drumsets/<Key> Kit.tmpl`: the four kits of [`KITS`], built in code and written with
//! [`write_drumset`], so a pick goes through the same `read_drumset` and `load_drumset` as any `.tmpl`.
//!
//! Details:
//! - A slot is `<Key> <Pad>` (the small line under the pad name) on folder `Built-in/<Key>/<Pad>`, with no `PTH1`: the
//!   engine synthesizes it (`honeydew_audio::builtin_drums`). Slots are in pad order and `PADS` is the identity (the
//!   kit is built the way [`DrumKit::default_kit`] is), so hex pad `i` plays slot `i` on key 48 + `i`.
//! - Closed and Open Hat share cut group 1; no other pad is cut.
//! - [`MIX`] sets Level (dB) and Pan for each pad name and [`TRIM`] each kit's level; every other param stays at its
//!   default. Nothing is louder than -7 dB, for headroom (the chord test in `src/app/tests/drumsets.rs`).

use honeydew_audio::builtin_drums::KITS;

use super::{DRUMSETS, Entry};
use crate::flm::write_drumset;
use crate::project_files::DRUMSET_EXT;
use crate::song::{DrumKit, SamplerSlot};

/// A pad's Level in dB and Pan (0.5 centre), by pad name, before the kit's [`TRIM`]. The table sits 7 dB under the
/// default so that a chord of the main pads at velocity 1 stays under the device's clamp at 1.0 (each sample peaks at
/// -1 dBFS and a Kick, Snare, Clap and Closed Hat together add up to +6 dBFS at 0 dB). Crackle is the exception: a
/// quiet bed, kept audible.
const MIX: &[(&str, f32, f32)] = &[
    ("Kick", -7.0, 0.5),
    ("Hard Kick", -7.0, 0.5),
    ("Snare", -8.5, 0.5),
    ("Clap", -10.0, 0.46),
    ("Rim", -11.0, 0.42),
    ("Closed Hat", -13.0, 0.62),
    ("Open Hat", -12.0, 0.62),
    ("Shaker", -15.0, 0.68),
    ("Crackle", -8.0, 0.5),
    ("Tom", -9.0, 0.5),
    ("Low Tom", -9.0, 0.38),
    ("Mid Tom", -9.0, 0.5),
    ("High Tom", -9.0, 0.62),
    ("Cowbell", -12.0, 0.56),
    ("Cymbal", -12.0, 0.36),
    ("Crash", -12.0, 0.36),
    ("Ride", -13.0, 0.66),
    ("Metal", -13.0, 0.56),
    ("Noise Hit", -11.0, 0.5),
];

/// Each kit's trim in dB, added to every pad's [`MIX`] level so the kits sit alike: measured on the first 300 ms of
/// Kick, Snare and Closed Hat, the kits' means are within 0.5 dB and each pad within 2.6 dB across kits. The driven
/// Foundry has the most energy, the clean Pocket the least.
const TRIM: &[(&str, f32)] = &[
    ("Pocket", 0.0),
    ("Dust", -3.5),
    ("Relay", -1.5),
    ("Foundry", -7.0),
];

/// The Level value that reads `db` dB: the inverse of the Level law, dB = 66·v^0.6 − 60 (0 dB is 0.853124; the default
/// 0.8531 is its rounding). The channel Level and Send Level follow the same law, so the templates use it too.
pub fn level(db: f32) -> f32 {
    ((db + 60.0) / 66.0).powf(1.0 / 0.6)
}

/// The kit `key` of [`KITS`] (`None` for another name, or a pad [`MIX`] lacks: a caller with a key of [`KITS`] may
/// `.expect` it).
pub(super) fn kit(key: &str) -> Option<DrumKit> {
    let (key, pads) = KITS.iter().find(|(k, _)| *k == key)?;
    let (_, trim) = TRIM.iter().find(|(k, _)| k == key)?;
    let mut kit = DrumKit::default();
    for pad in *pads {
        let (_, db, pan) = (MIX.iter().find(|(name, ..)| name == pad)).copied()?;
        let mut slot = SamplerSlot {
            name: format!("{key} {pad}"),
            folder: format!("Built-in/{key}/{pad}"),
            cut_group: u8::from(matches!(*pad, "Closed Hat" | "Open Hat")),
            ..SamplerSlot::default()
        };
        slot.params[..2].copy_from_slice(&[level(db + trim), pan]);
        kit.insert_slot(kit.slots.len(), slot);
    }
    Some(kit)
}

pub(super) fn entries() -> Vec<Entry> {
    let one = |key: &'static str| {
        let rel = format!("{DRUMSETS}/{key} Kit.{DRUMSET_EXT}");
        Entry::new(rel, move || {
            let kit = kit(key).expect("a kit of KITS has a mix for each pad");
            write_drumset(&kit).expect("our kit writes")
        })
    };
    KITS.iter().map(|(key, _)| one(key)).collect()
}
