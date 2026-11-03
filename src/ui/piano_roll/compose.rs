//! The `Generate` panel's music ([`super::generate`]): a chord progression and a melody for the clip, in the project's
//! key and scale ([`super::notes::project_scale`]), so everything it writes is in scale.
//!
//! - A **progression** is one triad a bar, on the preset's scale degrees (1 = the tonic), in the octave from the tonic
//!   at 48 + the root; the Lo-fi preset adds the 7th.
//! - A **melody** is one note a beat, starting from the tonic an octave above the chords: the bar's chord tone on the
//!   beat, a step away from the note before it on the off-beats, a breath at the end of some bars.
//! - **Who leads**: the chords (`Lead::Chords`, a melody written over them), or the melody — the clip's own notes when
//!   it has any, the chords then fitted under them bar by bar (`Lead::Melody`).
//!
//! Everything is drawn from `seed`, so the same settings and seed give the same notes (as the Humanize panel's).

use crate::song::{Event, PPQ};

/// A progression preset: the scale degrees it walks (one a bar) and whether its triads take a 7th.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preset {
    Pop,
    Minor,
    Lofi,
    Epic,
    /// A walk of its own, drawn from the seed: `New seed` gives another progression.
    Random,
}

/// The presets in the panel's order, with their names.
pub(crate) const PRESETS: [(&str, Preset); 5] = [
    ("Pop", Preset::Pop),
    ("Minor", Preset::Minor),
    ("Lo-fi", Preset::Lofi),
    ("Epic", Preset::Epic),
    ("Random", Preset::Random),
];

impl Preset {
    /// The progression's degrees, one a bar: I–V–vi–IV and its kin.
    fn degrees(self) -> &'static [u8] {
        match self {
            Self::Pop => &[1, 5, 6, 4],
            Self::Minor => &[1, 6, 3, 7],
            Self::Lofi => &[2, 5, 1, 6],
            Self::Epic => &[1, 6, 4, 5],
            // Random has no table: it walks from the seed (`walk`).
            Self::Random => &[1],
        }
    }

    /// Lo-fi's triads take a 7th.
    fn seventh(self) -> bool {
        self == Self::Lofi
    }
}

/// Who leads: the chords, or the melody (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead {
    Chords,
    Melody,
}

/// What the panel asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    pub preset: Preset,
    /// Bars to write, 1, 2, 4 or 8.
    pub bars: u8,
    pub seed: u64,
    pub lead: Lead,
}

/// The semitone offsets of the scale's degrees from the root, ascending: 0…11, seven of them in a diatonic scale and
/// twelve in Chromatic.
fn offsets(mask: u16) -> Vec<i32> {
    (0..12).filter(|d| mask >> (11 - d) & 1 == 1).collect()
}

/// The key of scale degree `degree` (1 = the tonic), `octave` octaves above the tonic at `base`.
fn key_of(base: i32, offs: &[i32], degree: i32, octave: i32) -> u8 {
    let n = offs.len() as i32;
    let i = degree - 1 + octave * n;
    let semis = offs[i.rem_euclid(n) as usize] + 12 * i.div_euclid(n);
    (base + semis).clamp(0, 127) as u8
}

/// The key's pitch class.
fn pitch_class(key: u8) -> i32 {
    i32::from(key % 12)
}

/// A small xorshift generator, so that a seed gives the same notes on every platform.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // 0 would keep the generator there.
        Self(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// 0…1.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// 0…n − 1.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A note at `beat` beats into the clip, `len` long.
fn note(beat: f64, len: f64, key: u8, velocity: u8) -> Event {
    Event::Note {
        pos: (beat * f64::from(PPQ)).round() as u32,
        len_beats: len,
        key,
        velocity,
        pan: Event::DEFAULT_PAN,
        slide: false,
    }
}

/// [`Preset::Random`]'s progression: the tonic on the first bar, then a step or two up or down the scale, now and
/// then a leap to one of the home degrees (1, 4, 5, 6), and the last bar back on the tonic or the fifth — a cadence.
fn walk(bars: usize, degrees: usize, rng: &mut Rng) -> Vec<u8> {
    let n = degrees.max(1) as i32;
    let home = [1, 4, 5, 6];
    let mut out = Vec::with_capacity(bars);
    let mut at = 1i32;
    for bar in 0..bars {
        if bar > 0 {
            at = if rng.unit() < 0.3 {
                home[rng.below(home.len())].min(n)
            } else {
                (at + [-2, -1, 1, 2][rng.below(4)]).clamp(1, n)
            };
        }
        out.push(at.clamp(1, n) as u8);
    }
    if let Some(last) = out.last_mut() {
        *last = if rng.unit() < 0.5 { 1 } else { 5.min(n) as u8 };
    }
    out
}

/// The degrees the clip's notes suggest, one a bar: of the scale's triads, the one whose tones cover the bar's notes
/// best (the first on a tie). A bar without notes keeps the one before it, or the tonic.
fn fitted(notes: &[Event], offs: &[i32], bars: usize, bpb: f64) -> Vec<u8> {
    let degree_count = offs.len() as i32;
    let mut out = Vec::with_capacity(bars);
    let mut last = 1;
    for bar in 0..bars {
        let from = bar as f64 * bpb;
        let to = from + bpb;
        let at: Vec<i32> = notes
            .iter()
            .filter_map(|e| match e {
                Event::Note { pos, key, .. } => {
                    let beat = f64::from(*pos) / f64::from(PPQ);
                    (beat >= from && beat < to).then(|| pitch_class(*key))
                }
                _ => None,
            })
            .collect();
        if at.is_empty() {
            out.push(last);
            continue;
        }
        let mut best = (0, 1);
        for d in 1..=degree_count {
            let tones = [d, d + 2, d + 4].map(|s| offs[(s - 1).rem_euclid(degree_count) as usize]);
            let covered = at
                .iter()
                .filter(|pc| tones.iter().any(|t| (**pc - t).rem_euclid(12) == 0))
                .count();
            if covered > best.0 {
                best = (covered, d);
            }
        }
        last = best.1 as u8;
        out.push(last);
    }
    out
}

/// One triad (a 7th with `seventh`) per bar, each `bpb` beats from the bar's start, from the tonic at `base`.
fn triads(degrees: &[u8], seventh: bool, base: i32, offs: &[i32], bpb: f64) -> Vec<Event> {
    let mut out = Vec::new();
    for (bar, &d) in degrees.iter().enumerate() {
        let beat = bar as f64 * bpb;
        let steps: Vec<i32> = if seventh {
            vec![0, 2, 4, 6]
        } else {
            vec![0, 2, 4]
        };
        for step in steps {
            let key = key_of(base, offs, i32::from(d) + step, 0);
            out.push(note(beat, bpb, key, 168));
        }
    }
    out
}

/// A melody over `degrees`, `bpb` beats a bar, from the tonic at `base`: the bar's chord tone on the beat, a scale step
/// from the note before it on the off-beats, a breath at the end of some bars.
fn melody(degrees: &[u8], base: i32, offs: &[i32], bpb: f64, rng: &mut Rng) -> Vec<Event> {
    let mut out = Vec::new();
    let beats = bpb.round().max(1.0) as usize;
    let mut previous = 1;
    for (bar, &d) in degrees.iter().enumerate() {
        for k in 0..beats {
            let beat = bar as f64 * bpb + k as f64;
            // The last off-beat of every other bar is a breath.
            if k + 1 == beats && bar % 2 == 1 && rng.unit() < 0.6 {
                continue;
            }
            let strong = k % 4 == 0;
            let degree = if strong {
                let tones = [0, 2, 4];
                i32::from(d) + tones[rng.below(tones.len())]
            } else {
                let step = [-2, -1, 1, 2][rng.below(4)];
                previous + step
            };
            previous = degree;
            // An octave up from the chords, and one up again when the line runs off the top of its range.
            let key = key_of(base, offs, degree, 0);
            let key = if key > 96 {
                key_of(base, offs, degree, -1)
            } else {
                key
            };
            let len = (0.5 + 0.25 * rng.unit()).min(bpb);
            let velocity = 190 + rng.below(41) as u8;
            out.push(note(beat, len, key, velocity));
        }
    }
    out
}

/// `events` sorted by tick, then by key.
fn sorted(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by_key(|e| match e {
        Event::Note { pos, key, .. } => (*pos, *key),
        _ => (0, 0),
    });
    events
}

/// The notes `Apply` writes into `clip`, in the project's key and scale `(root, mask)`, `bpb` beats a bar.
pub(crate) fn generate(
    s: &Settings,
    (root, mask): (u8, u16),
    clip: &[Event],
    bpb: f64,
) -> Vec<Event> {
    let offs = offsets(mask);
    if offs.is_empty() || bpb <= 0.0 {
        return clip.to_vec();
    }
    let bars = usize::from(s.bars.max(1));
    let mut rng = Rng::new(s.seed);
    let existing: Vec<Event> = clip
        .iter()
        .filter(|e| matches!(e, Event::Note { .. }))
        .copied()
        .collect();
    // The chords sit around the tonic at 48 + the root, the melody an octave above them.
    let (chord_base, melody_base) = (48 + i32::from(root % 12), 60 + i32::from(root % 12));
    let played = s.preset.degrees();
    // A Random progression is drawn before the melody, so its seed decides both; it takes 7ths now and then.
    let (degrees, seventh) = if s.preset == Preset::Random {
        (walk(bars, offs.len(), &mut rng), rng.unit() < 0.4)
    } else {
        let degrees = (0..bars).map(|bar| played[bar % played.len()]).collect();
        (degrees, s.preset.seventh())
    };
    let (degrees, melody) = if s.lead == Lead::Melody && !existing.is_empty() {
        (fitted(&existing, &offs, bars, bpb), existing)
    } else {
        (
            degrees.clone(),
            melody(&degrees, melody_base, &offs, bpb, &mut rng),
        )
    };
    let mut out = triads(&degrees, seventh, chord_base, &offs, bpb);
    out.extend(melody);
    sorted(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C major: the tones of degrees 0, 2, 4, 5, 7, 9 and 11 from C.
    const C_MAJOR: u16 = 2773;

    fn settings(preset: Preset, bars: u8, seed: u64, lead: Lead) -> Settings {
        Settings {
            preset,
            bars,
            seed,
            lead,
        }
    }

    fn keys(events: &[Event]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Note { key, .. } => Some(*key),
                _ => None,
            })
            .collect()
    }

    /// The chord notes: velocity 168, as [`triads`] writes them.
    fn chord_keys(events: &[Event]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Note {
                    key, velocity: 168, ..
                } => Some(*key),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_progression_is_the_presets_degrees_one_triad_a_bar_from_the_tonic() {
        let offs = offsets(C_MAJOR);
        // I–V–vi–IV: C major, G major, A minor, F major, a bar each.
        let chords = triads(&[1, 5, 6, 4], false, 48, &offs, 4.0);
        assert_eq!(
            keys(&chords),
            [48, 52, 55, 55, 59, 62, 57, 60, 64, 53, 57, 60]
        );
        let ticks: Vec<u32> = chords
            .iter()
            .map(|e| match e {
                Event::Note { pos, .. } => *pos,
                _ => unreachable!(),
            })
            .collect();
        // One bar = 4 beats = 512 ticks of the song's 128 a beat.
        assert_eq!(
            ticks,
            [0, 0, 0, 512, 512, 512, 1024, 1024, 1024, 1536, 1536, 1536]
        );
        // The Lo-fi preset's triads take a 7th.
        assert_eq!(triads(&[1], true, 48, &offs, 4.0).len(), 4);
    }

    #[test]
    fn everything_generated_is_in_scale_and_a_seed_gives_the_same_notes() {
        let s = settings(Preset::Pop, 2, 7, Lead::Chords);
        let a = generate(&s, (0, C_MAJOR), &[], 4.0);
        assert_eq!(a, generate(&s, (0, C_MAJOR), &[], 4.0), "the same seed");
        assert_ne!(
            a,
            generate(
                &settings(Preset::Epic, 2, 8, Lead::Chords),
                (0, C_MAJOR),
                &[],
                4.0
            )
        );
        let offs = offsets(C_MAJOR);
        for key in keys(&a) {
            let pc = i32::from(key % 12);
            assert!(
                offs.iter().any(|o| (pc - o).rem_euclid(12) == 0),
                "{key} is out of C major"
            );
        }
        // Two bars: six chord notes, and a melody over them.
        assert_eq!(chord_keys(&a).len(), 6);
        assert!(keys(&a).len() > 6);
        // Eight bars of a 2-bar progression: still one triad a bar.
        let long = generate(
            &settings(Preset::Pop, 8, 7, Lead::Chords),
            (0, C_MAJOR),
            &[],
            4.0,
        );
        assert_eq!(chord_keys(&long).len(), 24);
    }

    #[test]
    fn the_random_preset_walks_the_scale_from_the_tonic_and_a_seed_changes_it() {
        let offs = offsets(C_MAJOR);
        let mut walks = Vec::new();
        for seed in 0..6u64 {
            let s = settings(Preset::Random, 4, seed, Lead::Chords);
            let out = generate(&s, (0, C_MAJOR), &[], 4.0);
            let chords = chord_keys(&out);
            assert!(chords.len() >= 12, "seed {seed}: {chords:?}");
            // The first bar is the tonic's triad, and the whole walk is in scale.
            assert_eq!(chords[..3], [48, 52, 55], "seed {seed}");
            for key in keys(&out) {
                let pc = i32::from(key % 12);
                assert!(
                    offs.iter().any(|o| (pc - o).rem_euclid(12) == 0),
                    "seed {seed}: {key} is out of C major"
                );
            }
            walks.push(chords);
        }
        assert!(
            walks.windows(2).any(|w| w[0] != w[1]),
            "a fresh walk for every seed"
        );
    }

    #[test]
    fn a_melody_that_leads_keeps_its_notes_and_the_chords_are_fitted_under_them() {
        // C and E over one bar of C major: the triad under them is the tonic's.
        let clip = [note(0.0, 1.0, 60, 200), note(1.0, 1.0, 64, 200)];
        let out = generate(
            &settings(Preset::Pop, 1, 3, Lead::Melody),
            (0, C_MAJOR),
            &clip,
            4.0,
        );
        for e in clip {
            assert!(out.contains(&e), "{e:?} lost");
        }
        assert_eq!(chord_keys(&out), [48, 52, 55]);
        // A melody on G and B is covered by G major's triad and by E minor's, and the first one wins: E G B.
        let clip = [note(0.0, 1.0, 67, 200), note(1.0, 1.0, 71, 200)];
        let out = generate(
            &settings(Preset::Pop, 1, 3, Lead::Melody),
            (0, C_MAJOR),
            &clip,
            4.0,
        );
        assert_eq!(chord_keys(&out), [52, 55, 59]);
    }
}
