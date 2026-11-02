//! Our song templates through the app's own paths: each `Templates/<Name>.songtmpl` of the factory
//! reads back with no notices, builds every instrument in `SongAudioEngine` with no sample missing, and plays its two
//! bars from the start through `AudioRenderer` (no device). The ProjectFiles route (`.songtmpl` → untitled
//! `New Song (n)`) is tested in `src/content/templates.rs`. The factory folder holds exactly the nine template
//! names, and the newer four (Amapiano, Drift Phonk, Drill, Trap) keep the line-ups and the idioms their file sets.
//!
//! Levels: a render of the 2-bar loop played twice peaks in [−30, −1] dBFS, max(|L|, |R|): [−30, 0) with 1 dB of
//! margin, so a template never sits at the clip. Every part sounds: each channel's strip peaks above −40 dBFS. The
//! listen table prints with `--nocapture`.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use honeydew::audio_engine::SongAudioEngine;
use honeydew::content::{ROOT, TEMPLATES, factory};
use honeydew::flm::chunk::ContentPath;
use honeydew::flm::read_with_notices;
use honeydew::sample_cache::SampleCache;
use honeydew::song::{ChannelKind, Event, LaneKind, PPQ, Song, SynthKind};
use honeydew::ui::main_menu::SongLibrary;
use honeydew_audio::device::AudioRenderer;

const RATE: f32 = 48_000.0;

/// What a genre template plays over its bass: chords, or a lead line and no chords. A setup plays nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Harmony {
    None,
    Chords,
    Lead,
}

/// Our templates in menu order (byte order): name, tempo, and what it plays over its bass.
const OURS: [(&str, f64, Harmony); 9] = [
    ("Amapiano", 113.0, Harmony::Chords),
    ("Audio Recording", 120.0, Harmony::None),
    ("Boom Bap", 90.0, Harmony::Chords),
    ("D&B", 174.0, Harmony::Chords),
    ("Drift Phonk", 130.0, Harmony::Lead),
    ("Drill", 142.0, Harmony::Lead),
    ("Lo-Fi", 80.0, Harmony::Chords),
    ("Techno", 128.0, Harmony::Chords),
    ("Trap", 140.0, Harmony::Lead),
];

fn path(name: &str) -> PathBuf {
    Path::new(ROOT)
        .join(TEMPLATES)
        .join(format!("{name}.songtmpl"))
}

/// Template `name`, read from the factory as the app reads a `.songtmpl`; it must have no notices.
fn template(name: &str) -> Song {
    let bytes = factory().read(&path(name)).expect("the factory template");
    let (song, notices) = read_with_notices(&bytes).expect("a template that reads");
    assert_eq!(notices, Vec::<String>::new(), "{name}");
    song
}

/// The samples `song` plays, loaded as the app loads them: ours are made in code, so nothing is read.
fn samples(song: &Song) -> SampleCache {
    let mut cache = SampleCache::default();
    cache.load(song, &|_: &ContentPath| None);
    cache
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// `bars` bars of `song` from its start (a genre template's play loop wraps every two): the stereo output and each
/// channel's strip peak (instance 0), held over the render.
fn play_bars(song: &Song, bars: u32) -> (Vec<f32>, Vec<f32>) {
    let mut engine = SongAudioEngine::with_samples(song, RATE, &samples(song)).unwrap();
    engine.play();
    let frames = (f64::from(4 * bars) * 60.0 / song.tempo_bpm * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let mut strips = vec![0.0_f32; song.channels.len()];
    let mut block = vec![0.0; 512 * 2];
    while out.len() < frames * 2 {
        let n = 512.min(frames - out.len() / 2);
        engine.render(&mut block[..n * 2], 2);
        out.extend_from_slice(&block[..n * 2]);
        for i in 0..engine.meter_count() {
            let key = engine.meter_key(i);
            if key.instance_id == 0 {
                let m = engine.meter(i);
                let strip = &mut strips[key.channel];
                *strip = strip.max(m.peak_left).max(m.peak_right);
            }
        }
    }
    (out, strips)
}

#[test]
fn the_factory_templates_folder_lists_ours() {
    let listed = factory()
        .list(Some(&Path::new(ROOT).join(TEMPLATES)))
        .unwrap();
    for (name, ..) in OURS {
        let row = listed.iter().find(|e| e.path == path(name));
        assert!(row.is_some_and(|e| !e.is_folder), "{name}: {listed:?}");
    }
}

#[test]
fn the_factory_templates_folder_holds_exactly_the_nine_menu_names_in_order() {
    let listed = factory()
        .list(Some(&Path::new(ROOT).join(TEMPLATES)))
        .unwrap();
    assert!(listed.iter().all(|e| !e.is_folder), "{listed:?}");
    // The menu sorts the stems by byte order.
    let mut stems: Vec<String> = (listed.iter())
        .map(|e| {
            assert_eq!(e.path.extension().unwrap(), "songtmpl", "{e:?}");
            e.path.file_stem().unwrap().to_string_lossy().into_owned()
        })
        .collect();
    stems.sort();
    let want = [
        "Amapiano",
        "Audio Recording",
        "Boom Bap",
        "D&B",
        "Drift Phonk",
        "Drill",
        "Lo-Fi",
        "Techno",
        "Trap",
    ];
    assert_eq!(stems, want);
    assert_eq!(OURS.map(|(name, ..)| name), want);
}

#[test]
fn each_template_reads_back_with_no_notices_and_the_same_bytes_each_time() {
    for (name, bpm, harmony) in OURS {
        let genre = harmony != Harmony::None;
        let song = template(name);
        assert_eq!(song.tempo_bpm, bpm, "{name}");
        assert_eq!(song.master().kind, ChannelKind::Master, "{name}");
        // Play loops a genre template's two bars; a setup has no loop.
        let play_loop = genre.then_some((0, 8 * PPQ));
        assert_eq!(song.play_loop, play_loop, "{name}");
        let again = factory().read(&path(name)).unwrap();
        assert_eq!(again, factory().read(&path(name)).unwrap(), "{name}");
    }
}

#[test]
fn audio_recording_is_the_master_and_one_audio_record_channel_with_no_clips() {
    let song = template("Audio Recording");
    let kinds: Vec<ChannelKind> = song.channels.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ChannelKind::Master, ChannelKind::AudioRecord]);
    let lanes = song.channels.iter().flat_map(|c| &c.lanes);
    assert!(lanes.flat_map(|l| &l.clips).next().is_none());
}

#[test]
fn each_genre_template_has_a_drum_pattern_a_bass_line_and_its_harmony_in_bar_long_clips() {
    let bar = 4 * PPQ;
    for (name, _, harmony) in OURS {
        if harmony == Harmony::None {
            continue;
        }
        let song = template(name);
        let clips: Vec<_> = (song.channels.iter())
            .flat_map(|c| c.lanes.iter().map(move |l| (c, l)))
            .flat_map(|(c, l)| l.clips.iter().map(move |clip| (c, l, clip)))
            .collect();
        for (c, _, clip) in &clips {
            let len = clip.loop_ticks().unwrap();
            assert_eq!(clip.start_tick, 0, "{name} {}", c.name);
            assert!(
                [bar, 2 * bar].map(f64::from).contains(&len),
                "{name} {}: {len} ticks",
                c.name
            );
            assert!(clip.visible_ticks() <= f64::from(2 * bar), "{name}");
            assert!(!clip.events.is_empty(), "{name} {}", c.name);
            // A two-bar clip plays in both bars: it has an event in each, so a part never loses its bar 2.
            if len == f64::from(2 * bar) {
                let early = clip.events.iter().any(|e| e.pos() < bar);
                let late = clip.events.iter().any(|e| e.pos() >= bar);
                assert!(early && late, "{name} {}: events in both bars", c.name);
            }
        }
        // Drums: a kit of ours, with kick and snare (or clap) hits.
        let drums = (clips.iter())
            .find(|(c, l, _)| c.kind == ChannelKind::Drums && l.kind == LaneKind::Drums);
        let (c, _, clip) = drums.unwrap_or_else(|| panic!("{name}: a drum clip"));
        let kit = c.drum_kit.as_ref().unwrap();
        assert!(
            kit.slots.iter().all(|s| s.folder.starts_with("Built-in/")),
            "{name}"
        );
        let pads: Vec<&str> = (clip.events.iter())
            .filter_map(|e| match *e {
                Event::Drum { pad, .. } => kit.slot_on_pad(usize::from(pad)).map(|s| s.pad_name()),
                _ => None,
            })
            .collect();
        assert!(pads.contains(&"Kick"), "{name}: {pads:?}");
        assert!(
            pads.iter().any(|p| matches!(*p, "Snare" | "Clap")),
            "{name}: {pads:?}"
        );
        // A bass line (a run of single notes below C3) and chords (three or more notes at once).
        let notes = |clip: &honeydew::song::Clip| -> Vec<(u32, u8)> {
            (clip.events.iter())
                .filter_map(|e| match *e {
                    Event::Note { pos, key, .. } => Some((pos, key)),
                    _ => None,
                })
                .collect()
        };
        let synths = clips
            .iter()
            .filter(|(c, ..)| matches!(c.kind, ChannelKind::Synth(_)));
        let parts: Vec<Vec<(u32, u8)>> = synths.map(|(_, _, clip)| notes(clip)).collect();
        let bass = parts
            .iter()
            .any(|n| n.len() >= 4 && n.iter().all(|&(_, k)| k < 48));
        let chords = parts
            .iter()
            .any(|n| (n.iter()).any(|&(pos, _)| n.iter().filter(|&&(p, _)| p == pos).count() >= 3));
        // Or a lead line: four notes or more, all above the bass's range.
        let lead = parts
            .iter()
            .any(|n| n.len() >= 4 && n.iter().all(|&(_, k)| k >= 48));
        assert!(bass, "{name}: a bass line, {parts:?}");
        match harmony {
            Harmony::Chords => assert!(chords, "{name}: chords, {parts:?}"),
            Harmony::Lead => assert!(!chords && lead, "{name}: a lead and no chords, {parts:?}"),
            Harmony::None => unreachable!(),
        }
    }
}

#[test]
fn each_template_builds_every_instrument_with_no_sample_missing() {
    for (name, ..) in OURS {
        let song = template(name);
        let cache = samples(&song);
        assert_eq!(cache.missing().count(), 0, "{name}");
        let engine = SongAudioEngine::with_samples(&song, RATE, &cache).unwrap();
        // Every channel's strip and every module of its rack is in the engine.
        for (i, c) in song.channels.iter().enumerate() {
            let ids = std::iter::once(0).chain(c.rack.modules.iter().map(|m| m.instance_id));
            for id in ids {
                let built = (0..engine.meter_count()).any(|m| {
                    engine.meter_key(m).channel == i && engine.meter_key(m).instance_id == id
                });
                assert!(built, "{name}: channel {} module {id}", c.name);
            }
        }
        let synths = song
            .channels
            .iter()
            .filter(|c| matches!(c.kind, ChannelKind::Synth(_)));
        for c in synths {
            let ChannelKind::Synth(kind) = c.kind else {
                unreachable!()
            };
            assert_eq!(c.rack.modules[0].type_id, kind.type_id(), "{name}");
            assert_ne!(
                c.rack.modules[0].file.preset_name, "",
                "{name}: a preset of ours"
            );
            assert_ne!(kind, SynthKind::DwSampler, "{name}: P1 instruments only");
        }
    }
}

#[test]
fn each_template_plays_its_loop_twice_with_every_part_sounding_and_a_peak_in_range() {
    let mut failed = Vec::new();
    for (name, _, harmony) in OURS {
        let genre = harmony != Harmony::None;
        let song = template(name);
        // Twice round the 2-bar loop: the second pass rings on the first's tails and peaks higher.
        let (out, strips) = play_bars(&song, 4);
        assert!(out.iter().all(|s| s.is_finite()), "{name}: NaN or inf");
        let peak = db(out.iter().fold(0.0, |p, s| p.max(s.abs())));
        let rms = db((out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt());
        let parts: Vec<String> = (song.channels.iter().zip(&strips).skip(1))
            .map(|(c, &p)| format!("{} {:.1}", c.name, db(p)))
            .collect();
        println!(
            "{name:16} | peak {peak:6.1} dBFS | RMS {rms:6.1} dBFS | {}",
            parts.join(", ")
        );
        if !genre {
            // A setup with nothing to play is silent.
            if peak > -120.0 {
                failed.push(format!("{name}: not silent, {peak} dBFS"));
            }
            continue;
        }
        if !(-30.0..=-1.0).contains(&peak) {
            failed.push(format!("{name}: peak {peak:.1} dBFS"));
        }
        for (c, &p) in song.channels.iter().zip(&strips).skip(1) {
            if db(p) < -40.0 {
                failed.push(format!("{name}: {} is silent ({:.1} dBFS)", c.name, db(p)));
            }
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// The mix of each template, in menu order: (name, a reverb send, Chorus on the keys, swing, closed-hat rolls).
const MIX: [(&str, bool, bool, bool, bool); 9] = [
    ("Amapiano", true, false, true, false),
    ("Audio Recording", false, false, false, false),
    ("Boom Bap", true, false, true, false),
    ("D&B", true, false, false, false),
    ("Drift Phonk", false, false, false, false),
    ("Drill", false, false, false, true),
    ("Lo-Fi", true, true, true, false),
    ("Techno", false, false, false, false),
    ("Trap", true, false, false, true),
];

#[test]
fn the_reverb_sends_and_the_chorus_are_where_each_template_has_them() {
    for (name, reverb, chorus, ..) in MIX {
        let song = template(name);
        let sends: Vec<_> = (song.channels.iter())
            .filter(|c| c.kind == ChannelKind::EffectTrack)
            .collect();
        if !reverb {
            assert!(sends.is_empty(), "{name}: no send channel");
        } else {
            // One `Reverb` send channel holding Reverb 2 (35), wet only.
            assert_eq!(sends.len(), 1, "{name}");
            let track = sends[0];
            assert_eq!(track.name, "Reverb", "{name}");
            let modules: Vec<u16> = track.rack.modules.iter().map(|m| m.type_id).collect();
            assert_eq!(modules, [35], "{name}");
            assert_eq!(track.rack.modules[0].params[7], 0.0, "{name}: DRY 0");
            // A part sends to it besides the master, enabled, at −16 … −1 dB (Send Level 0.45 … 0.86).
            let sending = (song.channels.iter().skip(1)).filter(|c| {
                c.file.sends.get(1).is_some_and(|send| {
                    send.target_channel_id == track.id
                        && send.enabled > 0.5
                        && (0.45..=0.86).contains(&send.level)
                })
            });
            assert!(sending.count() >= 1, "{name}: a part sends to the reverb");
        }
        let has_chorus = (song.channels.iter())
            .filter(|c| matches!(c.kind, ChannelKind::Synth(_)))
            .any(|c| c.rack.modules.iter().any(|m| m.type_id == 11));
        assert_eq!(has_chorus, chorus, "{name}: Chorus (11)");
    }
}

#[test]
fn a_swung_template_moves_its_odd_sixteenths_and_a_straight_one_keeps_the_grid() {
    let step = PPQ / 4;
    for (name, _, _, swing, rolls) in MIX {
        let song = template(name);
        let drums = song.channels.iter().find(|c| c.kind == ChannelKind::Drums);
        let Some(drums) = drums else {
            assert!(!swing && !rolls, "{name}");
            continue;
        };
        let lane = &drums.lanes[0];
        let kit = drums.drum_kit.as_ref().unwrap();
        let off_grid = (lane.clips[0].events.iter()).filter(|e| e.pos() % step != 0);
        // A roll's hits are the closed hat's, off the grid; every other hit is on it unless the lane swings.
        let (hat, other): (Vec<&Event>, Vec<&Event>) = off_grid.partition(|e| match **e {
            Event::Drum { pad, .. } => kit
                .slot_on_pad(usize::from(pad))
                .is_some_and(|s| s.pad_name() == "Closed Hat"),
            _ => false,
        });
        assert_eq!(lane.swing > 0.0, swing, "{name}: the lane's SWING");
        match swing {
            true => assert!(hat.len() + other.len() >= 3, "{name}: swung hits"),
            false => {
                assert_eq!(other.len(), 0, "{name}: {other:?}");
                assert_eq!(!hat.is_empty(), rolls, "{name}: rolls {hat:?}");
            }
        }
    }
}

// ---- Amapiano, Drift Phonk, Drill, Trap ----

/// A note of a synth channel's clip.
#[derive(Clone, Copy, Debug)]
struct Note {
    pos: u32,
    key: u8,
    /// Ticks.
    len: u32,
    velocity: u8,
    slide: bool,
}

/// The synth channels of `song`, with their kind and preset name.
fn synths(song: &Song) -> Vec<(SynthKind, &str)> {
    let synths = song.channels.iter().filter_map(|c| match c.kind {
        ChannelKind::Synth(kind) => Some((kind, c.rack.modules[0].file.preset_name.as_str())),
        _ => None,
    });
    synths.collect()
}

/// The index of the Drums channel.
fn drums_at(song: &Song) -> usize {
    (song.channels.iter())
        .position(|c| c.kind == ChannelKind::Drums)
        .unwrap()
}

/// The pad index of the pad named `pad` in the drum kit.
fn pad_of(song: &Song, pad: &str) -> u8 {
    let kit = song.channels[drums_at(song)].drum_kit.as_ref().unwrap();
    kit.slots.iter().position(|s| s.pad_name() == pad).unwrap() as u8
}

/// The hits of pad `pad` (by name) in the song's drum clip, as ticks from its start.
fn hits(song: &Song, pad: &str) -> Vec<u32> {
    let index = pad_of(song, pad);
    let clip = &song.channels[drums_at(song)].lanes[0].clips[0];
    let hits = clip.events.iter().filter_map(|e| match *e {
        Event::Drum { pos, pad, .. } if pad == index => Some(pos),
        _ => None,
    });
    hits.collect()
}

/// The notes of the synth channel `at` (an index among the song's synths).
fn notes(song: &Song, at: usize) -> Vec<Note> {
    let channel = (song.channels.iter())
        .filter(|c| matches!(c.kind, ChannelKind::Synth(_)))
        .nth(at)
        .unwrap();
    let notes = channel.lanes[0].clips[0].events.iter();
    let notes = notes.filter_map(|e| match *e {
        Event::Note {
            pos,
            key,
            len_beats,
            velocity,
            slide,
            ..
        } => Some(Note {
            pos,
            key,
            len: (len_beats * f64::from(PPQ)).round() as u32,
            velocity,
            slide,
        }),
        _ => None,
    });
    notes.collect()
}

/// The closed-hat rolls: runs of hits one triplet apart, as (hits, gap in ticks): 21 or 22 ticks apart are sixteenth
/// triplets (`PPQ / 6`, rounded) and 42 or 43 eighth triplets.
fn rolls(song: &Song) -> Vec<(usize, u32)> {
    let mut hats = hits(song, "Closed Hat");
    hats.sort_unstable();
    let (mut rolls, mut run): (Vec<_>, Option<(usize, u32)>) = (vec![], None);
    for pair in hats.windows(2) {
        let gap = match pair[1] - pair[0] {
            21 | 22 => Some(21),
            42 | 43 => Some(42),
            _ => None,
        };
        run = match (gap, run) {
            (Some(g), Some((n, kind))) if g == kind => Some((n + 1, kind)),
            (Some(g), run) => {
                rolls.extend(run);
                Some((2, g))
            }
            (None, run) => {
                rolls.extend(run);
                None
            }
        };
    }
    rolls.extend(run);
    rolls
}

/// `song` with only channel `keep` playing, and of its events only those `keep_event` accepts: for a solo render.
fn solo(song: &Song, keep: usize, keep_event: impl Fn(&Event) -> bool) -> Song {
    let mut solo = song.clone();
    for (i, channel) in solo.channels.iter_mut().enumerate() {
        for lane in &mut channel.lanes {
            if i != keep {
                lane.clips.clear();
            }
            for clip in &mut lane.clips {
                clip.events.retain(&keep_event);
            }
        }
    }
    solo
}

/// The left channel of an interleaved stereo render.
fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.chunks(2).map(|frame| frame[0]).collect()
}

/// The RMS of `x` below about 150 Hz (a 2nd-order Butterworth low-pass), in dB.
fn sub_db(x: &[f32]) -> f32 {
    let (sin, cos) = (TAU * 150.0 / RATE).sin_cos();
    let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
    let (b0, b1, a0) = ((1.0 - cos) / 2.0, 1.0 - cos, 1.0 + alpha);
    let (a1, a2) = (-2.0 * cos, 1.0 - alpha);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    let mut energy = 0.0;
    for &x0 in x {
        let y0 = (b0 * (x0 + x2) + b1 * x1 - a1 * y1 - a2 * y2) / a0;
        (x2, x1, y2, y1) = (x1, x0, y1, y0);
        energy += y0 * y0;
    }
    db((energy / x.len() as f32).sqrt())
}

/// A template's bass and its second part, as a kind and a preset name each.
type Synths<'a> = [(SynthKind, &'a str); 2];

#[test]
fn the_second_four_have_the_planned_line_ups_on_the_relay_kit() {
    use SynthKind::{GmSynth, MiniSynth, SuperSaw, ThreeXOsc, TransistorBass};
    // Name, synths in channel order, and whether the second part sends to a reverb channel.
    let want: [(&str, Synths, bool); 4] = [
        (
            "Amapiano",
            [(GmSynth, "Wah Pluck"), (MiniSynth, "Felt Keys")],
            true,
        ),
        (
            "Drift Phonk",
            [(TransistorBass, "Fuzz Line"), (ThreeXOsc, "Razor Glide")],
            false,
        ),
        (
            "Drill",
            [(GmSynth, "Iron Sub"), (MiniSynth, "Glass Tines")],
            false,
        ),
        (
            "Trap",
            [(GmSynth, "Snarl Bass"), (SuperSaw, "Skyline")],
            true,
        ),
    ];
    for (name, synths_want, send) in want {
        let song = template(name);
        assert_eq!(synths(&song), synths_want, "{name}");
        let drums = song.channels.iter().find(|c| c.kind == ChannelKind::Drums);
        let kit = drums.unwrap().drum_kit.as_ref().unwrap();
        let relay = (kit.slots.iter()).all(|s| s.folder.starts_with("Built-in/Relay/"));
        assert!(relay, "{name}: the Relay kit");
        // A reverb send: an Effect Track named Reverb that the second part sends to, at about -9 … -6 dB (Send Level
        // 0.55 … 0.75), and no other channel does.
        let reverb = (song.channels.iter()).find(|c| c.kind == ChannelKind::EffectTrack);
        let sends_to = |c: &&honeydew::song::Channel| {
            (c.file.sends.iter())
                .filter(|s| Some(s.target_channel_id) == reverb.map(|r| r.id))
                .map(|s| (s.enabled > 0.5, s.level))
                .collect::<Vec<_>>()
        };
        let senders: Vec<(&str, Vec<(bool, f32)>)> = (song.channels.iter())
            .map(|c| (c.name.as_str(), sends_to(&c)))
            .filter(|(_, sends)| !sends.is_empty())
            .collect();
        match (send, reverb) {
            (true, Some(r)) => {
                assert_eq!(r.name, "Reverb", "{name}");
                assert_eq!(senders.len(), 1, "{name}: {senders:?}");
                let (sender, sends) = &senders[0];
                assert_eq!(
                    *sender, synths_want[1].1,
                    "{name}: the keys or the lead sends"
                );
                let [(on, level)] = sends[..] else {
                    panic!("{name}: one send to the reverb, {sends:?}");
                };
                assert!(on && (0.55..=0.75).contains(&level), "{name}: {level}");
            }
            (false, None) => {}
            _ => panic!("{name}: reverb {reverb:?}"),
        }
    }
}

#[test]
fn amapiano_has_a_kick_on_every_beat_a_syncopated_rim_and_an_audibly_swung_16th_shaker() {
    let song = template("Amapiano");
    let kicks = hits(&song, "Kick");
    assert!(
        (0..8).all(|beat| kicks.contains(&(beat * PPQ))),
        "{kicks:?}"
    );
    // The rim stays off the beat: 3-3-2 against the kick.
    let rim = hits(&song, "Rim");
    assert!(
        rim.len() >= 9 && rim.iter().all(|t| t % PPQ != 0),
        "{rim:?}"
    );
    // A hat on every sixteenth, and the swing moves each odd one off the grid (by 5 ticks, about 21 ms).
    let hats = hits(&song, "Closed Hat");
    assert_eq!(hats.len(), 32);
    let late = hats.iter().filter(|t| **t % (PPQ / 4) != 0).count();
    assert!(late >= 16, "{late} swung hats, {hats:?}");
    assert!(song.channels[drums_at(&song)].lanes[0].swing > 0.0);
    // A log-drum line: short, spaced notes under a beat each.
    let bass = notes(&song, 0);
    assert!(
        bass.len() >= 12 && bass.iter().all(|n| n.len <= PPQ),
        "{bass:?}"
    );
}

#[test]
fn drift_phonk_rings_the_cowbell_on_the_leads_rhythm_over_a_bass_that_follows_the_kick() {
    let song = template("Drift Phonk");
    let starts = |at: usize| notes(&song, at).iter().map(|n| n.pos).collect::<Vec<_>>();
    let cowbell = hits(&song, "Cowbell");
    assert_eq!(cowbell, starts(1), "the cowbell plays the lead's rhythm");
    assert_eq!(hits(&song, "Kick"), starts(0), "the bass plays the kick's");
    // Dotted-eighth spacing: 3 sixteenths between hits, then 4.
    assert!(cowbell.windows(2).any(|w| w[1] - w[0] == 3 * PPQ / 4));
    assert!(song.tempo_bpm >= 130.0 && song.tempo_bpm < 131.0);
}

#[test]
fn drill_and_trap_are_half_time_and_roll_their_hats_in_triplets() {
    let bar = 4 * PPQ;
    // The rolls (hits, nominal gap in ticks): Drill's sixteenth triplets, six hits and the downbeat hat that they land
    // on (a triplet after the last), then its three eighth triplets; Trap's six sixteenth triplets.
    let want = [
        ("Amapiano", vec![]),
        ("Drift Phonk", vec![]),
        ("Drill", vec![(7, 21), (3, 42)]),
        ("Trap", vec![(6, 21)]),
    ];
    for (name, want) in want {
        let song = template(name);
        assert_eq!(
            rolls(&song),
            want,
            "{name}: {:?}",
            hits(&song, "Closed Hat")
        );
        if name == "Drill" || name == "Trap" {
            // The backbeat is on beat 3 of both bars, and only there.
            for pad in ["Snare", "Clap"] {
                let beats: Vec<u32> = hits(&song, pad).iter().map(|t| t % bar).collect();
                assert_eq!(beats, [2 * PPQ, 2 * PPQ], "{name} {pad}");
            }
        }
    }
}

#[test]
fn drills_808_slides_from_its_anchor_notes_at_their_velocity_on_a_glide_preset() {
    let song = template("Drill");
    let bass = notes(&song, 0);
    let slides: Vec<_> = bass.iter().filter(|n| n.slide).collect();
    assert_eq!(slides.len(), 2, "{bass:?}");
    for slide in slides {
        let anchor =
            (bass.iter()).find(|a| !a.slide && a.pos < slide.pos && slide.pos < a.pos + a.len);
        let anchor = anchor.unwrap_or_else(|| panic!("an anchor for the slide at {}", slide.pos));
        assert_ne!(anchor.key, slide.key, "a slide to the same key");
        assert_eq!(anchor.velocity, slide.velocity, "only the pitch moves");
        assert!(slide.len >= PPQ / 2, "a glide of two sixteenths or more");
    }
    let module = (song.channels.iter())
        .find(|c| c.kind == ChannelKind::Synth(SynthKind::GmSynth))
        .map(|c| &c.rack.modules[0])
        .unwrap();
    let spec = honeydew::params::module_spec(module.type_id).unwrap();
    let auto = spec.params.iter().position(|p| p.name == "Auto Portamento");
    assert_eq!(module.params[auto.unwrap()], 1.0, "auto portamento on");
}

#[test]
fn drills_slide_notes_raise_the_808s_pitch_in_the_render() {
    let song = template("Drill");
    let bass = song
        .channels
        .iter()
        .position(|c| matches!(c.kind, ChannelKind::Synth(_)));
    let bass = bass.unwrap();
    let with = solo(&song, bass, |_| true);
    let without = solo(&song, bass, |e| {
        !matches!(e, Event::Note { slide: true, .. })
    });
    // Rising zero crossings over sixteenths 7-10 of bar 1, where the first slide glides up and the note rings out.
    let rising = |song: &Song| {
        let left = left(&play_bars(song, 1).0);
        let step = (60.0 / song.tempo_bpm / 4.0 * f64::from(RATE)) as usize;
        let window = &left[7 * step..11 * step];
        window
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count()
    };
    let (slid, straight) = (rising(&with), rising(&without));
    assert!(
        slid as f64 >= 1.1 * straight as f64,
        "{slid} crossings with the slides, {straight} without"
    );
}

#[test]
fn drill_and_trap_let_the_808_lead_the_sub_band_over_the_kick() {
    for name in ["Drill", "Trap"] {
        let song = template(name);
        let kick = pad_of(&song, "Kick");
        let kick_only = solo(
            &song,
            drums_at(&song),
            |e| matches!(*e, Event::Drum { pad, .. } if pad == kick),
        );
        let bass = song
            .channels
            .iter()
            .position(|c| matches!(c.kind, ChannelKind::Synth(_)));
        let bass = solo(&song, bass.unwrap(), |_| true);
        let sub = |song: &Song| sub_db(&left(&play_bars(song, 2).0));
        let lead = sub(&bass) - sub(&kick_only);
        assert!(
            (0.5..=4.0).contains(&lead),
            "{name}: the 808 is {lead:.1} dB over the kick below 150 Hz"
        );
    }
}
