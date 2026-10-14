//! The factory Slicer loops: six drum loops of our own,
//! each a step pattern played on one of the factory kits ([`crate::builtin_drums::KITS`]) and rendered at run time.
//! No sample goes in; the patterns and levels below are ours. A CSTM `PATH` or slot folder `Built-in/Loops/<Name>`
//! plays `builtin(folder)` (`builtin_drums`), which calls [`loop_sample`].
//!
//! A loop is `bars` bars of 16th-note steps. Each row names one pad of the kit and gives one character per step
//! (spaces are ignored, so groups of four read as beats): `.` rests, `x` hits and `X` accents.
//!
//! Details:
//! - Length: exactly `bars · 4 · 60 / bpm` seconds, to the nearest frame; step `n` starts at `n / (16 · bars)` of it,
//!   rounded. There is no swing: the grid is straight.
//! - Mixing: the kit's one-shots are each normalized to −1 dBFS, so every pad has a level of its own ([`level`]) and a
//!   plain hit is [`PLAIN`] of an accent. The sum is then normalized to −1 dBFS.
//! - A pad plays one hit at a time, like a drum machine voice: a hit stops where the next hit of the same pad starts,
//!   and a closed hat also stops an open hat. A hit stopped early, or still ringing at the end of the loop, fades out
//!   over its last 5 ms. A tail never wraps round to the start.
//! - Slice markers: `meta.cues` holds the first frame of every step that has a hit in any row, and `meta.tempo`
//!   the bpm. `meta.beats` stays unset: the Slicer takes the length from the tempo (`slicer::auto_clip_beats`).
//! - Mono at [`RATE`]. Rendering takes a few milliseconds, is deterministic and runs on the app thread; the sample
//!   cache keeps the result.

use crate::builtin_drums::{RATE, builtin};
use crate::sample::{SampleData, SampleMeta};

/// A plain hit (`x`) as a fraction of an accent (`X`).
const PLAIN: f32 = 0.6;
/// −1 dBFS.
const PEAK: f32 = 0.891_250_9;
/// The last 5 ms of a hit that is stopped early fade to silence.
const FADE: usize = RATE as usize / 200;

/// One loop: `rows` are (pad of `kit`, 16 characters per bar).
pub struct Loop {
    pub name: &'static str,
    pub kit: &'static str,
    pub bpm: u32,
    pub bars: u32,
    pub rows: &'static [(&'static str, &'static str)],
}

/// The loops, in definition order; the picker lists index order.
pub const LOOPS: [Loop; 6] = [
    // Clean and tight: a funky kick against a straight backbeat.
    Loop {
        name: "Pocket Groove 92",
        kit: "Pocket",
        bpm: 92,
        bars: 2,
        rows: &[
            ("Kick", "X... ..x. ..x. ....  X... ...x ..x. ..x."),
            ("Snare", ".... X... .... X...  .... X... .... X..x"),
            ("Closed Hat", "x.xx X.x. x.xx X.x.  x.xx X.xx x.x. X.xx"),
            ("Clap", ".... .... .... ..x.  .... .... .... x..."),
        ],
    },
    // Lo-fi boom bap: ghost notes, an open hat, a tom run and vinyl dust.
    Loop {
        name: "Dust Break 84",
        kit: "Dust",
        bpm: 84,
        bars: 2,
        rows: &[
            ("Kick", "X... ..x. .x.. ....  X... ...x ..x. ...."),
            ("Snare", ".... X... .... X...  .... X... .... X.x."),
            ("Rim", "..x. .... .... ..x.  .... ..x. .... ...."),
            ("Closed Hat", "x.x. x.x. x.x. x...  x.x. x.x. x.x. x.x."),
            ("Open Hat", ".... .... .... ..x.  .... .... .... ...."),
            ("Shaker", "...x ...x ...x ...x  ...x ...x ...x ...x"),
            ("Tom", ".... .... .... ....  .... .... ...x .x.x"),
            ("Crackle", "X... .... .... ....  x... .... .... ...."),
        ],
    },
    // A bouncy groove on a long sub kick; the second bar ends in a falling tom run.
    Loop {
        name: "Relay Bounce 100",
        kit: "Relay",
        bpm: 100,
        bars: 2,
        rows: &[
            ("Kick", "X..x ..x. ..x. ....  X..x ..x. X... ..x."),
            ("Snare", ".... X... .... X...  .... X... .... X..."),
            ("Clap", ".... .... .... ....  .... x... .... x..."),
            ("Closed Hat", "X.x. x.x. X.x. x...  X.x. x.x. X.x. x.x."),
            ("Open Hat", ".... .... .... ..x.  .... .... .... ...."),
            ("Rim", ".... .x.. .... ...x  .... .... .x.. ...."),
            ("Cowbell", ".... .... .... ....  ..x. .... ..x. ...."),
            ("High Tom", ".... .... .... ....  .... .... .... .x.."),
            ("Mid Tom", ".... .... .... ....  .... .... .... ..x."),
            ("Low Tom", ".... .... .... ....  .... .... .... ...x"),
            ("Cymbal", "x... .... .... ....  .... .... .... ...."),
        ],
    },
    // Trap-style half time with hat rolls and a toms fill.
    Loop {
        name: "Relay Roll 140",
        kit: "Relay",
        bpm: 140,
        bars: 2,
        rows: &[
            ("Kick", "X..x .... ..x. ....  X... ..x. ..x. ...x"),
            ("Snare", ".... .... X... ....  .... .... X... ...."),
            ("Clap", ".... .... x... ....  .... .... x... ...."),
            ("Closed Hat", "X.x. x.x. X.x. xxxx  X.xx x.xx X.x. x.x."),
            ("Open Hat", ".... ...x .... ....  .... .... .... ...."),
            ("Rim", ".... .... .... ....  ..x. .... .... ...."),
            ("High Tom", ".... .... .... ....  .... .... .... x..."),
            ("Mid Tom", ".... .... .... ....  .... .... .... .x.."),
            ("Low Tom", ".... .... .... ....  .... .... .... ..x."),
        ],
    },
    // Four on the floor, offbeat open hats, a hard kick in the second bar, a short tom fill into the turn.
    Loop {
        name: "Foundry Drive 128",
        kit: "Foundry",
        bpm: 128,
        bars: 2,
        rows: &[
            ("Kick", "X... X... X... X...  .... .... .... ...."),
            ("Hard Kick", ".... .... .... ....  X... X... X... X..."),
            ("Clap", ".... X... .... X...  .... X... .... X..."),
            ("Closed Hat", ".... .... .... ....  ...x ...x ...x ...x"),
            ("Open Hat", "..x. ..x. ..x. ..x.  ..x. ..x. ..x. ..x."),
            ("Crash", "x... .... .... ....  X... .... .... ...."),
            ("Low Tom", ".... .... .... ....  .... .... .... ..x."),
            ("High Tom", ".... .... .... ....  .... .... .... .x.."),
            ("Rim", ".... .... .... ....  ...x .... ...x ...."),
            ("Shaker", "x.x. x.x. x.x. x.x.  x.x. x.x. x.x. x.x."),
            ("Metal", ".... .... ..x. ....  .... .... ..x. ...."),
            ("Noise Hit", ".... .... .... ....  .... .... .... ...x"),
        ],
    },
    // Two-step drum and bass: kicks and backbeat, ride quarters in the second bar.
    Loop {
        name: "Foundry Rush 172",
        kit: "Foundry",
        bpm: 172,
        bars: 2,
        rows: &[
            ("Kick", "X... .... ..x. ....  .... .... ..xx ...."),
            ("Hard Kick", ".... .... .... ....  X... .... .... ...."),
            ("Snare", ".... X... .... X...  .... X..x .x.. X..."),
            ("Closed Hat", "x.X. x.X. x.X. x...  x.X. x.X. x.X. x.X."),
            ("Open Hat", ".... .... .... ..x.  .... .... .... ...."),
            ("Ride", ".... .... .... ....  x... x... x... x..."),
            ("Crash", "x... .... .... ....  .... .... .... ...."),
            ("Noise Hit", ".... .... .... ....  .... .... .... ...X"),
        ],
    },
];

/// The loop `name` (one of [`LOOPS`]) as a sample, or `None` for any other name.
pub fn loop_sample(name: &str) -> Option<SampleData> {
    LOOPS.iter().find(|l| l.name == name)?.render()
}

/// How loud a pad sits against the others: its one-shot is at −1 dBFS whatever it is.
fn level(pad: &str) -> f32 {
    match pad {
        "Kick" | "Hard Kick" => 1.0,
        "Snare" => 0.9,
        "Clap" | "Tom" | "Low Tom" | "Mid Tom" | "High Tom" => 0.8,
        "Rim" => 0.55,
        "Noise Hit" => 0.5,
        "Cowbell" => 0.45,
        "Closed Hat" | "Open Hat" | "Cymbal" | "Ride" | "Crash" | "Metal" => 0.4,
        "Shaker" => 0.35,
        "Crackle" => 0.3,
        _ => 0.7,
    }
}

impl Loop {
    /// The length in frames: `bars · 4 · 60 / bpm` seconds, to the nearest frame.
    pub fn frames(&self) -> usize {
        (f64::from(self.bars) * 240.0 * f64::from(RATE) / f64::from(self.bpm)).round() as usize
    }

    fn step_frame(&self, step: usize) -> usize {
        (step as f64 * self.frames() as f64 / f64::from(16 * self.bars)).round() as usize
    }

    /// A row's hits as (frame, gain before the pad's level), or `None` for a row of the wrong length or a bad character.
    fn hits(&self, row: &str) -> Option<Vec<(usize, f32)>> {
        let cells: Vec<char> = row.chars().filter(|c| *c != ' ').collect();
        if cells.len() != 16 * self.bars as usize {
            return None;
        }
        let mut hits = Vec::new();
        for (step, cell) in cells.into_iter().enumerate() {
            match cell {
                '.' => {}
                'x' => hits.push((self.step_frame(step), PLAIN)),
                'X' => hits.push((self.step_frame(step), 1.0)),
                _ => return None,
            }
        }
        Some(hits)
    }

    /// The loop as a mono sample, or `None` when the kit or a pad is unknown, a row is malformed or nothing is hit.
    pub fn render(&self) -> Option<SampleData> {
        let frames = self.frames();
        let rows = (self.rows.iter())
            .map(|(pad, row)| Some((*pad, self.hits(row)?)))
            .collect::<Option<Vec<_>>>()?;
        let mut out = vec![0.0_f32; frames];
        for &(pad, ref hits) in &rows {
            let one = builtin(&format!("Built-in/{}/{pad}", self.kit))?.left;
            // A hit stops where the next hit of its pad starts; a closed hat also stops an open hat.
            let mut stops: Vec<usize> = (rows.iter())
                .filter(|(other, _)| *other == pad || (pad == "Open Hat" && *other == "Closed Hat"))
                .flat_map(|(_, hits)| hits.iter().map(|&(frame, _)| frame))
                .collect();
            stops.sort_unstable();
            for &(start, accent) in hits {
                let ring = start + one.len();
                let next = stops.iter().copied().find(|&stop| stop > start);
                let end = next.map_or(ring, |stop| stop.min(ring)).min(frames);
                let gain = accent * level(pad);
                for (i, sample) in one[..end - start].iter().enumerate() {
                    let fade = if end < ring {
                        ((end - 1 - start - i) as f32 / (FADE - 1) as f32).min(1.0)
                    } else {
                        1.0
                    };
                    out[start + i] += gain * fade * sample;
                }
            }
        }
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        if peak == 0.0 {
            return None;
        }
        out.iter_mut().for_each(|s| *s *= PEAK / peak);
        let mut cues: Vec<u32> = (rows.iter())
            .flat_map(|(_, hits)| hits.iter().map(|&(frame, _)| frame as u32))
            .collect();
        cues.sort_unstable();
        cues.dedup();
        Some(SampleData {
            sample_rate: RATE,
            left: out.into(),
            right: None,
            meta: SampleMeta {
                tempo: Some(self.bpm as f32),
                cues,
                ..Default::default()
            },
        })
    }
}
