//! Our song templates from SONGS `New`: the menu lists them, and each opens as a new song
//! whose every sample is in the app's cache and which plays.

use std::path::Path;

use honeydew_audio::device::AudioRenderer;

use super::*;
use crate::audio_engine::SongAudioEngine;
use crate::content::{ROOT, TEMPLATES, factory};
use crate::ui::main_menu::SongLibrary;

/// Ours, with their tempos: the nine names, in the menu's order.
const OURS: [(&str, f64); 9] = [
    ("Amapiano", 113.0),
    ("Audio Recording", 120.0),
    ("Boom Bap", 90.0),
    ("D&B", 174.0),
    ("Drift Phonk", 130.0),
    ("Drill", 142.0),
    ("Lo-Fi", 80.0),
    ("Techno", 128.0),
    ("Trap", 140.0),
];

/// The stems the menu lists after its `Templates` header: the factory's, by byte order.
fn listed() -> Vec<String> {
    let folder = Path::new(ROOT).join(TEMPLATES);
    let mut stems: Vec<String> = (factory().list(Some(&folder)).unwrap().into_iter())
        .filter_map(|e| Some(e.path.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    stems.sort();
    stems
}

#[test]
fn the_new_menu_opens_each_of_our_templates_and_it_plays() {
    let mut rig = Rig::new();
    rig.app.files.factory = factory();
    // The factory folder is exactly the nine, so the menu lists all nine.
    assert_eq!(listed(), OURS.map(|(name, _)| name));
    for (name, bpm) in OURS {
        rig.open_menu(MenuTab::Songs);
        rig.click_id(songs_id(("big", 0_usize)));
        rig.frame(vec![]);
        // The menu lists ours after its `Templates` header, by byte order.
        let texts = rig.texts();
        let at = |t: &str| texts.iter().position(|x| x == t).unwrap_or(usize::MAX);
        assert!(
            OURS.iter().all(|(n, _)| at("Templates") < at(n)),
            "{texts:?}"
        );
        assert!(
            OURS.windows(2).all(|w| at(w[0].0) < at(w[1].0)),
            "{texts:?}"
        );
        let row = 2 + listed().iter().position(|s| s == name).unwrap();
        rig.click_id(songs_id("new").with((0_usize, row)));
        rig.frame(vec![]);
        let song = rig.app.song.clone();
        assert_eq!(song.tempo_bpm, bpm, "{name}");
        assert!(song.title.starts_with("New Song"), "{name}: {}", song.title);
        assert!(!rig.app.files.modified(), "{name}");
        assert_eq!(rig.take(), ["Stop", "ReplaceSong"], "{name}");
        let samples = &rig.app.audio.samples;
        assert_eq!(samples.missing().count(), 0, "{name}");
        // It plays: a bar from the start, on the app's own samples.
        let mut engine = SongAudioEngine::with_samples(&song, 48_000.0, samples).unwrap();
        engine.play();
        let mut out = vec![0.0; (4.0 * 60.0 / bpm * 48_000.0) as usize * 2];
        engine.render(&mut out, 2);
        let peak = out.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        match name {
            "Audio Recording" => assert_eq!(peak, 0.0, "{name}: nothing to play"),
            _ => assert!(peak > 0.03, "{name}: {peak}"),
        }
    }
}
