//! Our demo song: `Paper Lanterns`, 16 bars of our own music on our kits and presets, built with the
//! templates' [`Builder`] and written with [`flm::write`]. `Restore demo songs` and the app's first run put it into
//! `My Songs/Demo songs/` through the songs library ([`restore_demo_songs`]); SONGS lists that folder and opens the song
//! as any `.flm`.
//!
//! Details:
//! - The song: D minor at 108 BPM, Dm7, Bbmaj7, Fadd9 and C over four bars. Relay drums, a MiniSynth bass, MiniSynth
//!   keys and a SuperSaw lead, the keys and lead sending to a Reverb 2 send channel. The arrangement places the parts'
//!   clips as an intro (bars 1–4: keys, the drums from bar 3), a main part (5–12: everything), a break (13–14: keys and
//!   lead) and an outro (15–16: drums, bass and keys). Play loops the 16 bars.
//! - As demos ship: the metadata lock (META@776) on and time counting off (META@256 −1); it
//!   was never saved ("Never / demo project"), and its title (HEAD and META@520) is the file's name.
//! - The GUIDs are zero, as the templates' ([`Builder::build`]), so the demo is the same bytes every time.
//! - Levels as the templates': the 16 bars peak in [−30, −1] dBFS, and each part sounds in every clip
//!   (`tests/content_demo.rs`).
//! - The folder is `Demo songs`. With Original Content present SONGS also lists
//!   its read-only `Demo Songs`; ours is `Demo songs` in My Songs.
//! - Both make the folder when it is missing, and the user's other files in it stay. The first run writes only what is
//!   missing: when the settings were lost (`demoSongsRestored` false again) the user's copy stays. `Restore demo songs`
//!   writes ours over a file of the same name, moving the user's copy to Recently Deleted first when it differs (as
//!   `Delete` does).
//! - The web has no songs folder: its library is this session's, in memory, so the first-run hook stores the demos into
//!   it at every start, and `Restore demo songs` stores them again into the same session library.

use std::io;

use super::templates::{Builder, chords};
use crate::flm;
use crate::song::{PPQ, Song, SynthKind};
use crate::ui::main_menu::{SongEntry, SongLibrary, delete};

/// The folder below the songs library's root the demos go in.
pub const DEMO_FOLDER: &str = "Demo songs";

/// Our demo's title and file stem.
const TITLE: &str = "Paper Lanterns";

/// Ticks a bar (4/4).
const BAR: u32 = 4 * PPQ;

/// Our demo songs, each as its file name and `.flm` bytes.
pub fn demo_songs() -> Vec<(String, Vec<u8>)> {
    let bytes = flm::write(&paper_lanterns()).expect("our demo writes");
    vec![(format!("{TITLE}.flm"), bytes)]
}

/// Writes each demo into [`DEMO_FOLDER`] below `library`'s root, making the folder. A file of the same name is the
/// user's copy: the first run (`replace` false) leaves it, as settings that were lost must not cost the user an edited
/// copy; `Restore demo songs` (`replace` true) replaces it, first moving it to Recently Deleted unless it holds our
/// bytes already. Fails when the library has no root (no songs folder), or a write or the move fails.
pub fn restore_demo_songs(library: &mut dyn SongLibrary, replace: bool) -> io::Result<()> {
    let root = library.root().ok_or(io::ErrorKind::NotFound)?;
    let folder = root.join(DEMO_FOLDER);
    if !library.exists(&folder) {
        library.make_folder(&folder)?;
    }
    for (name, bytes) in demo_songs() {
        let path = folder.join(&name);
        if library.exists(&path) {
            if !replace || library.read(&path).is_ok_and(|b| b == bytes) {
                continue;
            }
            let copy = SongEntry {
                name: name.clone(),
                path,
                is_folder: false,
                recency: 0,
            };
            delete(library, &[copy], &mut Vec::new()).map_err(io::Error::other)?;
        }
        library.store_in(&folder, &name, &bytes)?;
    }
    Ok(())
}

/// Each bar's chord, keys voicing and bass root: Dm7, Bbmaj7, Fadd9, C.
const CHORDS: [&[u8]; 4] = [
    &[57, 60, 62, 65],
    &[58, 62, 65, 69],
    &[57, 60, 65, 67],
    &[55, 60, 64, 67],
];
const ROOTS: [u8; 4] = [38, 34, 41, 36];

/// The lead over the four bars: (sixteenth, key, length in sixteenths, velocity).
const LEAD: [(u32, u8, u32, u8); 17] = [
    (0, 69, 4, 200),
    (4, 72, 2, 180),
    (6, 74, 6, 210),
    (12, 77, 2, 190),
    (14, 76, 2, 170),
    (16, 74, 6, 210),
    (22, 72, 2, 170),
    (24, 74, 2, 180),
    (26, 77, 6, 210),
    (32, 76, 4, 200),
    (36, 72, 2, 170),
    (38, 69, 6, 200),
    (44, 72, 2, 170),
    (46, 74, 2, 180),
    (48, 76, 6, 210),
    (54, 74, 2, 180),
    (56, 72, 6, 200),
];

fn paper_lanterns() -> Song {
    // The bass, each bar: the root pushed along in eighths and sixteenths, a fourth below on the and of three.
    let bass: Vec<(u32, u8, u32, u8)> = (0..4)
        .flat_map(|bar| {
            let (at, root) = (bar * 16, ROOTS[bar as usize]);
            [
                (0, 3, 0, 230),
                (3, 3, 0, 170),
                (6, 2, 0, 200),
                (10, 2, 5, 180),
                (12, 4, 0, 210),
            ]
            .map(|(step, len, below, velocity)| (at + step, root - below, len, velocity))
        })
        .collect();
    // The keys, each bar: the chord on one, the and of two and three's second sixteenth.
    let comp: Vec<(u32, u32, &[u8])> = (0..4)
        .flat_map(|bar| {
            [(0, 5), (6, 4), (10, 6)]
                .map(|(step, len)| (bar * 16 + step, len, CHORDS[bar as usize]))
        })
        .collect();
    let mut song = Builder::new(108.0)
        .drums(
            "Relay",
            &[
                ("Kick", "X.......X.x.....X.......X.....x."),
                ("Snare", "....X.......X.......X.......X..."),
                ("Rim", ".......x...............x........"),
                ("Closed Hat", "x.x.x.x.x.x.x.x.x.x.x.x.x.x.x..."),
                ("Open Hat", "..............................x."),
            ],
        )
        .at(&[(2, 2), (4, 8), (14, 2)])
        .level(-2.0)
        .synth(SynthKind::MiniSynth, "Velvet Sub", 64, &bass)
        .at(&[(4, 8), (14, 2)])
        .level(-4.0)
        .synth(SynthKind::MiniSynth, "Glass Tines", 64, &chords(&comp))
        .at(&[(0, 4), (4, 8), (12, 2), (14, 2)])
        .level(-5.0)
        .reverb_send(-8.0)
        .synth(SynthKind::SuperSaw, "Skyline", 64, &LEAD)
        .at(&[(4, 8), (12, 2)])
        .level(-10.0)
        .reverb_send(-8.0)
        .build();
    song.play_loop = Some((0, 16 * BAR));
    song.title = TITLE.into();
    let meta = &mut song.file.meta;
    meta.display_title = TITLE.into();
    meta.locked = true;
    meta.editing_seconds = -1.0;
    song
}
