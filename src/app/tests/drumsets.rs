//! The factory drumsets: four kits of ours, `Drumsets/<Key> Kit.tmpl` in the factory ([`crate::content`]), read back
//! through the app's own loader and picked from `Load Drumset`.

use std::path::Path;

use eframe::egui::{Event, Id};
use honeydew_audio::builtin_drums::KITS;
use honeydew_audio::device::AudioRenderer;

use super::rack_wiring::picker_row;
use super::*;
use crate::content::{self, DRUMSETS, ROOT};
use crate::flm::{read_drumset, write_drumset};
use crate::project_files::mem::MemFiles;
use crate::project_files::{DRUMSET_EXT, ProjectFiles};
use crate::sample_cache::{SampleCache, SampleKey, reader};
use crate::song::{DrumKit, SamplerSlot};
use crate::ui::main_menu::SongLibrary;

/// The root rows in order: `My Files`, `Default`, then ours, `A-Z`.
const ROWS: [&str; 6] = [
    "My Files",
    "Default",
    "Dust Kit",
    "Foundry Kit",
    "Pocket Kit",
    "Relay Kit",
];

fn factory_kit(key: &str) -> DrumKit {
    let path = Path::new(ROOT)
        .join(DRUMSETS)
        .join(format!("{key} Kit.{DRUMSET_EXT}"));
    let bytes = content::factory().read(&path).expect("the factory kit");
    read_drumset(&bytes).expect("a kit that reads")
}

/// The drums channel (1) of a rig on the web's files (so no user folder), and the synth (2) selected.
fn drums_on_the_web() -> Rig {
    let mut rig = Rig::with_drums_and_synth();
    rig.app.files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    rig
}

fn kit(rig: &Rig) -> DrumKit {
    rig.app.song.channels[1].drum_kit.clone().unwrap()
}

/// The drums track's menu, `Load Drumset` (column 1, row 0): the `Drumsets` picker opens on channel 1.
fn load_drumset(rig: &mut Rig) {
    let pos = header(1);
    rig.frame(vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        rig.frame(vec![Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    rig.frame(vec![]);
    rig.click_id(Id::new("track_menu").with((1_usize, 0_usize)));
    rig.frame(vec![]);
}

/// The peak of what channel 1's engine renders for pad `pad`'s key, struck at full velocity.
fn peak_of_pad(engine: &mut crate::audio_engine::SongAudioEngine, pad: usize) -> f32 {
    engine.note_on(1, 48 + pad as u8, 1.0);
    let mut out = vec![0.0; 8192 * 2];
    engine.render(&mut out, 2);
    out.iter().fold(0.0, |peak, s| s.abs().max(peak))
}

#[test]
fn each_factory_kit_reads_back_as_a_kit_on_our_built_in_pads() {
    assert_eq!(KITS.len(), ROWS.len() - 2);
    for (key, pads) in KITS {
        let kit = factory_kit(key);
        let n = kit.slots.len();
        assert!((4..=14).contains(&n), "{key}: {n} pads");
        assert_eq!(n, pads.len(), "{key}");
        // `PADS` is the identity: hex pad i plays slot i, whose key is 48 + i with no stored note.
        let identity: [u8; 22] = std::array::from_fn(|i| i as u8);
        assert_eq!(kit.file.pads, identity, "{key}");
        assert_eq!(kit.pad_note(n - 1), Some(48 + n as u8 - 1), "{key}");
        let mut distinct: Vec<&str> = kit.slots.iter().map(|s| s.pad_name()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), n, "{key}: pad names are distinct");
        // Ids 1..n; the kit's own `lastSlotId`.
        let ids: Vec<u32> = kit.slots.iter().map(|s| s.file.id).collect();
        assert_eq!(ids, (1..=n as u32).collect::<Vec<_>>(), "{key}");
        assert_eq!(kit.file.last_slot_id, n as u32, "{key}");
        let mut cache = SampleCache::default();
        let song = {
            let mut song = crate::song::Song::default();
            song.add_channel(crate::song::ChannelKind::Drums);
            song.channels[1].drum_kit = Some(kit.clone());
            song
        };
        let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
        cache.load(&song, &reader(&files));
        for (slot, pad) in kit.slots.iter().zip(pads) {
            assert_eq!(slot.name, format!("{key} {pad}"));
            assert_eq!(slot.folder, format!("Built-in/{key}/{pad}"));
            assert_eq!(slot.file.path, None, "{key} {pad}: no PTH1");
            let hat = matches!(*pad, "Closed Hat" | "Open Hat");
            assert_eq!(slot.cut_group, u8::from(hat), "{key} {pad}");
            assert!(
                matches!(SampleKey::of_slot(slot), Some(SampleKey::Builtin(_))),
                "{key} {pad}"
            );
            let sample = cache.slot(slot).unwrap_or_else(|| panic!("{key} {pad}"));
            assert!(sample.frames() > 0, "{key} {pad}");
            // Level and Pan are ours; every other param is at its default.
            let default = SamplerSlot::default().params;
            assert_eq!(slot.params[2..], default[2..], "{key} {pad}");
            assert!(
                slot.params[0] <= default[0],
                "{key} {pad}: no louder than 0 dB"
            );
        }
        assert_eq!(cache.missing().count(), 0, "{key}");
        // The file is the kit, bytes for bytes.
        assert_eq!(write_drumset(&kit).unwrap(), {
            let path = Path::new(ROOT)
                .join(DRUMSETS)
                .join(format!("{key} Kit.{DRUMSET_EXT}"));
            content::factory().read(&path).unwrap()
        });
    }
}

#[test]
fn the_drumsets_root_lists_my_files_default_and_the_four_kits() {
    let mut rig = drums_on_the_web();
    load_drumset(&mut rig);
    for _ in 0..3 {
        rig.frame(vec![]);
    }
    let listed: Vec<String> = rig
        .texts()
        .into_iter()
        .filter(|t| ROWS.contains(&t.as_str()))
        .collect();
    assert_eq!(listed, ROWS);
    let row = |i: usize| {
        rig.ctx
            .read_response(Id::new("preset_picker").with(("row", i)))
    };
    assert!(row(ROWS.len() - 1).is_some());
    assert!(row(ROWS.len()).is_none(), "nothing else is listed");
}

#[test]
fn picking_dust_kit_gives_the_channel_its_8_pads_in_one_undo_step() {
    let mut rig = drums_on_the_web();
    let before = rig.app.song.clone();
    rig.take();
    load_drumset(&mut rig);
    picker_row(&mut rig, 2);
    assert_eq!(rig.overlays(), []);
    // One new engine, no other command.
    assert_eq!(rig.take(), ["ReplaceSong adopting no-resync"]);
    let loaded = kit(&rig);
    let dust = factory_kit("Dust");
    assert_eq!(loaded.slots.len(), 8);
    let names: Vec<&str> = loaded.slots.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Dust Kick",
            "Dust Snare",
            "Dust Rim",
            "Dust Closed Hat",
            "Dust Open Hat",
            "Dust Shaker",
            "Dust Tom",
            "Dust Crackle"
        ]
    );
    // New ids after the default kit's last (8): ids are never reused.
    let ids: Vec<u32> = loaded.slots.iter().map(|s| s.file.id).collect();
    assert_eq!(ids, (9..=16).collect::<Vec<_>>());
    assert_eq!(loaded.file.pads, dust.file.pads);
    // Every sample is in the cache, and none is missing.
    let samples = &rig.app.audio.samples;
    assert!(loaded.slots.iter().all(|s| samples.slot(s).is_some()));
    assert_eq!(samples.missing().count(), 0);
    // One undo step gets the old kit back. Its pad ids stay taken: the kit's last id is not undone, so none comes back.
    let ctx = rig.ctx.clone();
    rig.app.step(&ctx, Step::Undo, 0);
    let mut want = before.channels.clone();
    want[1].drum_kit.as_mut().unwrap().file.last_slot_id = 16;
    assert_eq!(rig.app.song.channels, want);
}

#[test]
fn foundry_kit_pad_13_sounds_through_the_rig() {
    let mut rig = drums_on_the_web();
    load_drumset(&mut rig);
    picker_row(&mut rig, 3);
    assert_eq!(kit(&rig).slots.len(), 14);
    assert_eq!(kit(&rig).slots[13].name, "Foundry Noise Hit");
    // The drums channel is the current one: its pads are the keyboard band. Hex pad 13 is in the second pad row,
    // below the default 5 U panel: an 8 U panel brings it on screen (the panel takes a few frames to grow).
    rig.app.selected_channel = 1;
    rig.app.settings.preferred_kbd_size = 8.0;
    for _ in 0..10 {
        rig.frame(vec![]);
    }
    rig.take();
    let pad_centre = |i| super::editor_wiring::pad_in(8.0, i);
    rig.press(pad_centre(13));
    assert_eq!(rig.take(), ["NoteOn 1 61 127"]);
    // The pad is held while the engine renders.
    let mut out = vec![0.0; 8192 * 2];
    (rig.rec.0.borrow_mut().engine.as_mut().unwrap()).render(&mut out, 2);
    rig.release(pad_centre(13));
    let peak = out.iter().fold(0.0f32, |peak, s| s.abs().max(peak));
    // −40 dBFS is 0.01.
    assert!(peak > 0.01, "pad 13 peaks at {peak}");
}

#[test]
fn every_pad_of_every_factory_kit_sounds() {
    let mut rig = drums_on_the_web();
    for (row, (key, pads)) in [(2, KITS[1]), (3, KITS[3]), (4, KITS[0]), (5, KITS[2])] {
        load_drumset(&mut rig);
        picker_row(&mut rig, row);
        assert_eq!(kit(&rig).slots.len(), pads.len(), "{key}");
        let samples = &rig.app.audio.samples;
        let mut engine =
            crate::audio_engine::SongAudioEngine::with_samples(&rig.app.song, 44_100.0, samples)
                .expect("an engine");
        for (pad, name) in pads.iter().enumerate() {
            let peak = peak_of_pad(&mut engine, pad);
            assert!(peak > 0.01, "{key} {name}: peak {peak}");
        }
    }
}

/// A solo or chorded hit on `pads` of `kit` (at velocity 1, full-length pads): two seconds of stereo from the engine,
/// before the device's clamp.
fn hit_together(kit: &DrumKit, pads: &[usize]) -> Vec<f32> {
    let files = ProjectFiles::with_provider(Box::new(MemFiles::default()));
    let mut song = crate::song::Song::default();
    song.add_channel(crate::song::ChannelKind::Drums);
    song.channels[1].drum_kit = Some(kit.clone());
    let mut cache = SampleCache::default();
    cache.load(&song, &reader(&files));
    let mut engine =
        crate::audio_engine::SongAudioEngine::with_samples(&song, 44_100.0, &cache).unwrap();
    for &pad in pads {
        engine.note_on(1, 48 + pad as u8, 1.0);
    }
    let mut out = vec![0.0; 2 * 44_100 * 2];
    engine.render(&mut out, 2);
    out
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

fn pad_of(kit: &DrumKit, name: &str) -> Option<usize> {
    kit.slots.iter().position(|s| s.pad_name() == name)
}

/// The device clamps at 1.0, so a chord of the kit's main pads must not reach it: Kick, Snare, Clap and Closed Hat
/// (the pads a kit has) at velocity 1 peak at -0.5 dBFS or less.
#[test]
fn the_main_pads_hit_together_leave_half_a_db_of_headroom() {
    for (key, _) in KITS {
        let kit = factory_kit(key);
        let chord: Vec<usize> = ["Kick", "Snare", "Clap", "Closed Hat"]
            .iter()
            .filter_map(|name| pad_of(&kit, name))
            .collect();
        assert!(chord.len() >= 3, "{key}");
        let out = hit_together(&kit, &chord);
        let peak = db(out.iter().fold(0.0f32, |peak, s| peak.max(s.abs())));
        assert!(peak <= -0.5, "{key}: the chord peaks at {peak:.1} dBFS");
    }
}

/// The kits' trims even them out: the first 300 ms of Kick, Snare and Closed Hat, in RMS dB. A kit's mean is within
/// 1 dB of the others', and each of the three pads within 3 dB across kits.
#[test]
fn the_kits_sit_within_a_few_db_of_each_other() {
    let rms = |kit: &DrumKit, name: &str| {
        let out = hit_together(kit, &[pad_of(kit, name).unwrap()]);
        let n = 300 * 44_100 / 1000 * 2;
        db((out[..n].iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt())
    };
    let pads = ["Kick", "Snare", "Closed Hat"];
    let table: Vec<[f32; 3]> = KITS
        .iter()
        .map(|(key, _)| pads.map(|name| rms(&factory_kit(key), name)))
        .collect();
    let spread = |values: Vec<f32>| {
        let (lo, hi) = (
            values.iter().copied().fold(f32::MAX, f32::min),
            values.iter().copied().fold(f32::MIN, f32::max),
        );
        hi - lo
    };
    let means = table
        .iter()
        .map(|row| row.iter().sum::<f32>() / 3.0)
        .collect();
    assert!(spread(means) <= 1.0, "{table:?}");
    for (i, name) in pads.iter().enumerate() {
        let spread = spread(table.iter().map(|row| row[i]).collect());
        assert!(
            spread <= 3.0,
            "{name}: {spread:.1} dB across kits, {table:?}"
        );
    }
}

/// The factory kit table, pad by pad: Level in dB and Pan in percent (negative is
/// left), and each kit's trim in dB, added to its pads' levels.
const LEVELS: &[(&str, f32, i32)] = &[
    ("Kick", -7.0, 0),
    ("Hard Kick", -7.0, 0),
    ("Snare", -8.5, 0),
    ("Clap", -10.0, -8),
    ("Rim", -11.0, -16),
    ("Closed Hat", -13.0, 24),
    ("Open Hat", -12.0, 24),
    ("Shaker", -15.0, 36),
    ("Crackle", -8.0, 0),
    ("Tom", -9.0, 0),
    ("Low Tom", -9.0, -24),
    ("Mid Tom", -9.0, 0),
    ("High Tom", -9.0, 24),
    ("Cowbell", -12.0, 12),
    ("Cymbal", -12.0, -28),
    ("Crash", -12.0, -28),
    ("Ride", -13.0, 32),
    ("Metal", -13.0, 12),
    ("Noise Hit", -11.0, 0),
];
const TRIMS: &[(&str, f32)] = &[
    ("Pocket", 0.0),
    ("Dust", -3.5),
    ("Relay", -1.5),
    ("Foundry", -7.0),
];

#[test]
fn every_pad_of_every_kit_reads_the_documented_level_and_pan() {
    use crate::params::DRUM_SLOT_PARAMS;
    for (key, pads) in KITS {
        let kit = factory_kit(key);
        let trim = TRIMS.iter().find(|(k, _)| *k == key).unwrap().1;
        for (slot, pad) in kit.slots.iter().zip(pads) {
            let (_, db, pan) = *LEVELS.iter().find(|(name, ..)| name == pad).unwrap();
            let pan = match pan {
                0 => "center".to_owned(),
                p if p < 0 => format!("{}% left", -p),
                p => format!("{p}% right"),
            };
            let level = format!("{:.1} dB", db + trim);
            let read = |i: usize| DRUM_SLOT_PARAMS[i].display_text(slot.params[i]);
            assert_eq!((read(0), read(1)), (level, pan), "{key} {pad}");
        }
    }
}
