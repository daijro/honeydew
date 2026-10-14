//! Our song templates II: Amapiano, Drift Phonk, Drill and Trap, each a [`Builder`] song on our Relay kit (an analog
//! machine with a long sub kick, tuned toms and a cowbell), chained into [`super::templates`]. Patterns, lines and
//! levels are our own; the names are from the menu's nine.
//!
//! Details:
//! - Every clip is 2 bars (32 sixteenths), with the same grid and the same GUID, level and loop rules as the other
//!   templates.
//! - The genres' hallmarks are in the data, and `tests/content_templates.rs` pins them:
//!   - Amapiano (113 BPM, D minor, a shuffle of swing 0.3): a kick on every beat, a syncopated 3-3-2 rim, 16th hats
//!     with the "e" accented, and a short, plucky bass line (the log drum's place) under rootless Dm9 and Bbmaj9 stabs.
//!   - Drift Phonk (130 BPM, C minor): the kick and the bass share one pattern, and the Cowbell and the lead share
//!     another (3-3-3-3-4), under a gliding lead.
//!   - Drill (142 BPM, G minor): half-time (the snare on beat 3), sparse 8th hats with triplet rolls, and an 808 that
//!     slides (slide notes) under a bell that plays Gm and F arpeggios.
//!   - Trap (140 BPM, F minor): half-time, 8th hats that burst into 16ths, a triplet roll, a growling 808 on the
//!     descending Db, C, Bb, and a slow lead through the reverb send.
//! - A roll is `n` hits of one pad from a tick on (not on the sixteenth grid): triplet sixteenths in this song are
//!   `PPQ / 6` ticks apart, rounded. A slide note takes its anchor's velocity, so only the pitch moves.
//! - In Drill and Trap the 808 leads the sub band (below 150 Hz): the Relay kick's pad Level is 3 dB lower than the
//!   kit's, in the template only (`trim_pad`), so the kit itself is unchanged.
//! - There is no chord part in the three that have a lead: the harmony is the bass's roots and the lead's notes
//!   (`tests/content_templates.rs` asks for chords or a lead line).

use super::{Builder, Template, chords, level};
use crate::song::{Event, PPQ, Song, SynthKind};

/// Every template here plays this kit.
const KIT: &str = "Relay";

/// The templates of this file.
pub(super) fn templates() -> Vec<Template> {
    vec![
        ("Amapiano", amapiano),
        ("Drift Phonk", drift_phonk),
        ("Drill", drill),
        ("Trap", trap),
    ]
}

impl Builder {
    /// `n` hits of `pad` on the last (Drums) channel, from tick `at`, `gap` ticks apart (rounded), the velocity going
    /// from `vel.0` to `vel.1`.
    fn roll(mut self, pad: &str, at: u32, n: u32, gap: f64, vel: (u8, u8)) -> Self {
        let kit = (self.last().drum_kit.as_ref()).expect("a drums channel");
        let pad = kit.slots.iter().position(|s| s.pad_name() == pad);
        let pad = pad.expect("a pad of the kit") as u8;
        let events = &mut self.last().lanes[0].clips[0].events;
        for i in 0..n {
            let t = if n > 1 {
                f64::from(i) / f64::from(n - 1)
            } else {
                0.0
            };
            let velocity = f64::from(vel.0) + t * (f64::from(vel.1) - f64::from(vel.0));
            events.push(Event::Drum {
                pos: at + (f64::from(i) * gap).round() as u32,
                len_beats: 0.0,
                pad,
                velocity: velocity.round() as u8,
                pan: Event::DEFAULT_PAN,
                value: Event::DEFAULT_PITCH,
                repeat: 0,
            });
        }
        events.sort_by_key(Event::pos);
        self
    }

    /// The Level of pad `pad` on the last (Drums) channel's kit moved by `db` dB (the Level law: dB = 66·v^0.6 − 60).
    fn trim_pad(mut self, pad: &str, db: f32) -> Self {
        let kit = (self.last().drum_kit.as_mut()).expect("a drums channel");
        let slot = kit.slots.iter_mut().find(|s| s.pad_name() == pad);
        let slot = slot.expect("a pad of the kit");
        slot.params[0] = level(66.0 * slot.params[0].powf(0.6) - 60.0 + db);
        self
    }

    /// Slide notes of (sixteenth, key, length in sixteenths, velocity) on the last (Notes) channel: each glides the
    /// note sounding at its start to its key over its length.
    fn slides(mut self, slides: &[(u32, u8, u32, u8)]) -> Self {
        for &(step, key, steps, velocity) in slides {
            let (pos, end) = (self.tick(step), self.tick(step + steps));
            let events = &mut self.last().lanes[0].clips[0].events;
            events.push(Event::Note {
                pos,
                len_beats: f64::from(end - pos) / f64::from(PPQ),
                key,
                velocity,
                pan: Event::DEFAULT_PAN,
                slide: true,
            });
            events.sort_by_key(Event::pos);
        }
        self
    }
}

/// A 113 BPM shuffle in D minor: a four-on-the-floor kick, a syncopated rim and shaker-like hats, a plucky bass line
/// and piano-like stabs over Dm9 and Bbmaj9.
fn amapiano() -> Song {
    // Rootless voicings: F A C E, and D F A C over the bass's Bb.
    const DM9: &[u8] = &[53, 57, 60, 64];
    const BBMAJ9: &[u8] = &[50, 53, 57, 60];
    Builder::new(113.0)
        .swing(0.3)
        .drums(
            KIT,
            &[
                ("Kick", "X...x...x...x...X...x...x...x.x."),
                ("Clap", "....X.......X.......X.......X.x."),
                ("Closed Hat", "xXxxxXxxxXxxxXxxxXxxxXxxxXxxxXxx"),
                ("Open Hat", "..x...x...x...x...x...x...x...x."),
                ("Rim", "...x..x....x..x....x..x....x.x.x"),
            ],
        )
        .level(-3.0)
        .synth(
            SynthKind::GmSynth,
            "Wah Pluck",
            32,
            &[
                (0, 38, 2, 230),
                (3, 38, 1, 180),
                (6, 45, 2, 210),
                (8, 41, 2, 200),
                (11, 38, 1, 170),
                (14, 36, 2, 200),
                (16, 34, 2, 230),
                (19, 34, 1, 180),
                (22, 41, 2, 210),
                (24, 38, 2, 200),
                (27, 34, 1, 170),
                (30, 36, 2, 200),
            ],
        )
        .level(-5.0)
        .synth(
            SynthKind::MiniSynth,
            "Felt Keys",
            32,
            &chords(&[
                (0, 3, DM9),
                (6, 2, DM9),
                (10, 2, DM9),
                (14, 2, DM9),
                (16, 3, BBMAJ9),
                (22, 2, BBMAJ9),
                (26, 2, BBMAJ9),
                (30, 2, BBMAJ9),
            ]),
        )
        .level(-13.0)
        .reverb_send(-9.0)
        .build()
}

/// A 130 BPM phonk in C minor: the Cowbell rings the lead's 3-3-3-3-4 rhythm over a fuzzed bass that follows the kick.
fn drift_phonk() -> Song {
    Builder::new(130.0)
        .drums(
            KIT,
            &[
                ("Kick", "X.....x.x..x....X.....x.x.....x."),
                ("Snare", "....X.......X.......X.......X..."),
                ("Clap", "....x.......x.......x.......x..."),
                ("Closed Hat", "x.x.x.x.x.x.x.xxx.x.x.x.x.x.xxxx"),
                ("Cowbell", "X..x..x..x..x...X..x..x..x..x..."),
                ("Cymbal", "X..............................."),
            ],
        )
        .level(-4.0)
        .synth(
            SynthKind::TransistorBass,
            "Fuzz Line",
            32,
            &[
                (0, 36, 5, 230),
                (6, 36, 2, 200),
                (8, 39, 3, 220),
                (11, 34, 5, 210),
                (16, 32, 5, 230),
                (22, 32, 2, 200),
                (24, 34, 5, 220),
                (30, 31, 2, 200),
            ],
        )
        .level(-2.0)
        .synth(
            SynthKind::ThreeXOsc,
            "Razor Glide",
            32,
            &[
                (0, 72, 2, 220),
                (3, 75, 2, 210),
                (6, 74, 2, 210),
                (9, 72, 2, 210),
                (12, 70, 4, 220),
                (16, 68, 2, 220),
                (19, 72, 2, 210),
                (22, 75, 2, 210),
                (25, 74, 2, 210),
                (28, 72, 4, 220),
            ],
        )
        .level(-12.0)
        .build()
}

/// A 142 BPM drill in G minor: the snare on beat 3, hats that roll in triplets, an 808 that slides, and a bell
/// arpeggio over Gm and F.
fn drill() -> Song {
    // Triplet sixteenths and eighths, in ticks.
    let (sixteenth, eighth) = (f64::from(PPQ) / 6.0, f64::from(PPQ) / 3.0);
    let bar = 4 * PPQ;
    Builder::new(142.0)
        .drums(
            KIT,
            &[
                ("Kick", "X.....x....x....X.....x....x..x."),
                ("Snare", "........X...............X......."),
                ("Clap", "........x...............x......."),
                ("Closed Hat", "x.x.x.x.x.x.....x.x.x.x.x.x....."),
                ("Open Hat", "..............................x."),
            ],
        )
        .trim_pad("Kick", -3.0)
        .roll("Closed Hat", 3 * PPQ, 6, sixteenth, (110, 255))
        .roll("Closed Hat", bar + 3 * PPQ, 3, eighth, (150, 255))
        .level(-2.0)
        .synth(
            SynthKind::GmSynth,
            "Iron Sub",
            32,
            &[
                (0, 31, 8, 230),
                (11, 29, 5, 220),
                (16, 31, 9, 230),
                (27, 34, 3, 210),
                (30, 31, 2, 210),
            ],
        )
        .slides(&[(6, 34, 2, 230), (22, 36, 3, 230)])
        .level(-11.0)
        .synth(
            SynthKind::MiniSynth,
            "Glass Tines",
            32,
            &[
                (0, 67, 3, 200),
                (3, 70, 3, 190),
                (6, 74, 2, 190),
                (8, 70, 3, 190),
                (11, 67, 3, 190),
                (14, 65, 2, 180),
                (16, 65, 3, 200),
                (19, 69, 3, 190),
                (22, 72, 2, 190),
                (24, 69, 3, 190),
                (27, 65, 3, 190),
                (30, 62, 2, 180),
            ],
        )
        .level(0.0)
        .build()
}

/// A 140 BPM trap in F minor: a clap on beat 3, hats that burst and roll, a growling 808 stepping down Db, C, Bb, and
/// a slow supersaw line through the reverb.
fn trap() -> Song {
    let sixteenth = f64::from(PPQ) / 6.0;
    Builder::new(140.0)
        .drums(
            KIT,
            &[
                ("Kick", "X.........x..x..X..x......x....."),
                ("Snare", "........x...............x......."),
                ("Clap", "........X...............X......."),
                ("Closed Hat", "x.xxx.x.x.xxx.x.x.x.x.xxx.x....."),
                ("Open Hat", "...............x................"),
            ],
        )
        .trim_pad("Kick", -3.0)
        .roll("Closed Hat", 4 * PPQ + 3 * PPQ, 6, sixteenth, (110, 255))
        .level(-2.0)
        .synth(
            SynthKind::GmSynth,
            "Snarl Bass",
            32,
            &[
                (0, 29, 9, 235),
                (10, 32, 3, 200),
                (13, 29, 3, 200),
                (16, 37, 3, 230),
                (19, 36, 7, 210),
                (26, 34, 6, 215),
            ],
        )
        .level(-10.5)
        .synth(
            SynthKind::SuperSaw,
            "Skyline",
            32,
            &[
                (0, 72, 6, 210),
                (6, 75, 2, 190),
                (8, 77, 4, 220),
                (14, 75, 2, 180),
                (16, 73, 6, 210),
                (22, 72, 2, 190),
                (24, 68, 6, 210),
                (30, 72, 2, 180),
            ],
        )
        .level(-11.0)
        .reverb_send(-8.0)
        .build()
}
