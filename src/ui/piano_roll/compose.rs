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

use crate::song::scales::SCALE_MASKS;
use crate::song::{Event, PPQ};

/// The keys the panel offers, `KEYS`' index being the pitch class.
pub(crate) const KEYS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The scales the panel offers: ids into [`crate::song::scales::SCALES`], the common ones (a picker tapped
/// through wants few).
pub(crate) const SCALE_CHOICES: [usize; 16] =
    [0, 12, 17, 18, 6, 7, 19, 9, 11, 3, 14, 22, 32, 27, 21, 20];

/// The dominant note durations, in beats: where the melody's notes fall.
pub(crate) const DURATIONS: [(&str, f64); 6] = [
    ("1/16", 0.25),
    ("1/8", 0.5),
    ("1/4", 1.0),
    ("1/2", 2.0),
    ("1 bar", 4.0),
    ("2 bars", 8.0),
];

/// The register, as the absolute window it names: octave 0 is C4..B4 (60..71), and "Very high" clamps to C5.
pub(crate) const OCTAVES: [(&str, i32); 5] = [
    ("Very low", 36),
    ("Low", 48),
    ("Medium", 60),
    ("High", 72),
    ("Very high", 72),
];

/// The chance a slot is left out, as a rest.
pub(crate) const RESTS: [(&str, f64); 3] = [("None", 0.0), ("Default", 0.18), ("More", 0.4)];

/// The arpeggio modes: none, a sixteenth-note run, or a ~1/6-beat run.
pub(crate) const ARPS: [(&str, f64); 3] = [("None", 0.0), ("Standard", 1.0), ("Fast", 2.0)];

/// Which chords to write.
pub(crate) const CHORD_MODES: [&str; 5] = [
    "Chords + melody",
    "Rolled + melody",
    "Chords only",
    "Rolled only",
    "No chords",
];

/// What the panel asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    /// Bars to write, 1, 2, 4 or 8.
    pub bars: u8,
    pub seed: u64,
    /// The panel's key, a pitch class; `None` follows the project's.
    pub key: Option<u8>,
    /// The panel's scale, an id into [`crate::song::scales::SCALES`]; `None` follows the project's.
    pub scale: Option<usize>,
    /// The register window's base key ([`OCTAVES`]).
    pub octave: i32,
    /// The dominant note duration, in beats ([`DURATIONS`]).
    pub duration: f64,
    /// The chance a slot is a rest ([`RESTS`]).
    pub rest: f64,
    /// The arpeggio mode ([`ARPS`]); 0 for none.
    pub arp: f64,
    /// Which chords to write ([`CHORD_MODES`]).
    pub chords: u8,
    /// The tempo `Apply` sets on the song, BPM; `None` keeps the project's.
    pub tempo: Option<f64>,
}

/// The semitone offsets of the scale's degrees from the root, ascending: 0…11, seven of them in a diatonic scale and
/// twelve in Chromatic.
fn offsets(mask: u16) -> Vec<i32> {
    (0..12).filter(|d| mask >> (11 - d) & 1 == 1).collect()
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

/// `events` sorted by tick, then by key.
fn sorted(mut events: Vec<Event>) -> Vec<Event> {
    events.sort_by_key(|e| match e {
        Event::Note { pos, key, .. } => (*pos, *key),
        _ => (0, 0),
    });
    events
}

/// The chance a slot becomes an arpeggio run.
const ARP_SLOT_PROB: f64 = 0.10;
/// The chance a slot gets chord tones with `Chords + melody` / `Rolled + melody`.
const CHORD_PROB: f64 = 0.15;
/// A per-note chance a repeated motif note drifts.
const MOTIF_MUTATION: f64 = 0.25;
/// The chance a fresh walk step snaps to the tonic.
const TONIC_GRAVITY: f64 = 0.08;
/// A roll's stagger per chord tone, in beats.
const ROLL_STAGGER: f64 = 0.03;
/// Every note's velocity, as midigen writes.
const VELOCITY: u8 = 100;

/// The notes `Apply` writes into `clip`, in the project's key and scale `(root, mask)`, `bpb` beats a bar — midigen's
/// generator: the dominant duration fills the piece in slots of one or two units, a short motif (the first bar's
/// slots, at least four) is repeated with small mutations, the walk has tonic gravity, and chords, arpeggio runs and
/// rests land as the panel says.
pub(crate) fn generate(
    s: &Settings,
    (root, mask): (u8, u16),
    clip: &[Event],
    bpb: f64,
) -> Vec<Event> {
    // The panel's key and scale win over the project's where it sets them.
    let root = s.key.unwrap_or(root);
    let mask = s
        .scale
        .map_or(mask, |i| SCALE_MASKS[i.min(SCALE_MASKS.len() - 1)]);
    let offs = offsets(mask);
    if offs.is_empty() || bpb <= 0.0 {
        return clip.to_vec();
    }
    let bars = usize::from(s.bars.max(1));
    let mut rng = Rng::new(s.seed);
    // The register window is absolute (the scale's root does not shift it): the panel's octave picks its base.
    let base = s.octave;
    let pcs: Vec<i32> = offs
        .iter()
        .map(|o| (i32::from(root % 12) + o).rem_euclid(12))
        .collect();
    let pitches: Vec<i32> = (base..base + 12)
        .filter(|p| pcs.contains(&p.rem_euclid(12)))
        .collect();
    if pitches.is_empty() {
        return clip.to_vec();
    }
    let unit = s.duration.max(1.0 / 16.0);
    let total = bars as f64 * bpb;
    // The slots: each one or two units long, the longer only when it fits.
    let mut slots: Vec<(f64, f64)> = Vec::new();
    let mut t = 0.0;
    while t < total {
        let mut dur = unit;
        if rng.unit() < 0.3 && t + 2.0 * unit <= total {
            dur = 2.0 * unit;
        }
        if t + dur > total {
            dur = total - t;
        }
        slots.push((t, dur));
        t += dur;
    }
    // The motif: the first bar's slots, at least four.
    let mut motif_len = 0;
    let mut acc = 0.0;
    for &(_, dur) in &slots {
        acc += dur;
        motif_len += 1;
        if acc >= bpb {
            break;
        }
    }
    let motif_len = motif_len.max(4).min(slots.len());
    // The tonic: the window's tone of the key's pitch class nearest C4.
    let tonic = pitches
        .iter()
        .copied()
        .filter(|p| p.rem_euclid(12) == i32::from(root % 12))
        .min_by_key(|p| (p - 60).abs())
        .unwrap_or(pitches[0]);
    let nearest = |p: i32| -> usize {
        pitches
            .iter()
            .enumerate()
            .min_by_key(|(_, k)| (*k - p).abs())
            .map(|(i, _)| i)
            .unwrap_or(0)
    };
    let clamp = |i: i32| -> usize { (i.max(0) as usize).min(pitches.len() - 1) };
    // The melody's pitch per slot: a walk with tonic gravity for the motif and most of the rest; the motif
    // repeated, mutated now and then, after.
    let mut idx = nearest(tonic);
    let mut motif: Vec<usize> = Vec::new();
    let mut notes: Vec<i32> = Vec::with_capacity(slots.len());
    for i in 0..slots.len() {
        let degree_idx = if i < motif_len || rng.unit() < 0.55 {
            if rng.unit() < TONIC_GRAVITY {
                nearest(tonic)
            } else {
                clamp(idx as i32 + [-2, -1, -1, 0, 1, 1, 2][rng.below(7)])
            }
        } else {
            let mut m = motif[i % motif_len];
            if rng.unit() < MOTIF_MUTATION {
                m = clamp(m as i32 + [-2, -1, 1, 2][rng.below(4)]);
            }
            m
        };
        idx = degree_idx;
        if i < motif_len {
            motif.push(degree_idx);
        }
        notes.push(pitches[degree_idx]);
    }
    // The arpeggio slots.
    let arp_slots: std::collections::HashSet<usize> = if s.arp > 0.0 {
        slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.1 >= unit && rng.unit() < ARP_SLOT_PROB)
            .map(|(i, _)| i)
            .collect()
    } else {
        Default::default()
    };
    // A diatonic tertian chord from scale degree `root_idx`: root, +2, +4, and +6 half the time.
    let chord_of = |root_idx: usize, seventh: bool| -> Vec<i32> {
        let n = pitches.len();
        let idxs = [root_idx, root_idx + 2, root_idx + 4]
            .into_iter()
            .chain((seventh).then_some(root_idx + 6));
        idxs.map(|ix| pitches[ix % n] + 12 * (ix / n) as i32)
            .collect()
    };
    let chords_only = s.chords == 2 || s.chords == 3;
    let rolled = s.chords == 1 || s.chords == 3;
    let mut out: Vec<Event> = Vec::new();
    for (i, &(at, dur)) in slots.iter().enumerate() {
        // A rest where the rests say.
        if i > 0 && rng.unit() < s.rest {
            continue;
        }
        // An arpeggio: a scale run filling the slot, a sixteenth at a time, or a sixth of a beat when fast.
        if arp_slots.contains(&i) {
            let step = if s.arp >= 2.0 { 1.0 / 6.0 } else { 0.25 };
            let n = ((dur / step).round() as usize).max(1);
            let dir = if rng.unit() < 2.0 / 3.0 { 1 } else { -1 };
            for j in 0..n {
                let p = pitches[clamp(idx as i32 + dir * j as i32)];
                out.push(note(
                    at + j as f64 * step,
                    (step * 0.9).min(dur),
                    p.clamp(0, 127) as u8,
                    VELOCITY,
                ));
            }
            continue;
        }
        if chords_only {
            // Every slot onset is a chord, weighted toward the tonic and dominant.
            let root = if rng.unit() < 0.6 {
                pitches[rng.below(pitches.len()).min(pitches.len() - 1)]
            } else {
                pitches[rng.below(pitches.len())]
            };
            let chord = chord_of(nearest(root), rng.unit() < 0.5);
            for (k, p) in chord.iter().enumerate() {
                let stagger = if rolled { k as f64 * ROLL_STAGGER } else { 0.0 };
                out.push(note(at + stagger, (dur - stagger).max(0.25), *p as u8, VELOCITY));
            }
            continue;
        }
        out.push(note(at, dur.min(bpb.max(unit)), notes[i] as u8, VELOCITY));
        // Chord tones on the onset now and then, staggered when rolled, without re-striking the melody's note.
        if s.chords < 4 && rng.unit() < CHORD_PROB {
            let chord = chord_of(nearest(notes[i]), rng.unit() < 0.5);
            for (k, p) in chord.iter().enumerate().skip(1) {
                let stagger = if rolled { k as f64 * ROLL_STAGGER } else { 0.0 };
                out.push(note(at + stagger, (dur - stagger).max(0.25), *p as u8, VELOCITY));
            }
        }
    }
    sorted(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C major: the tones of degrees 0, 2, 4, 5, 7, 9 and 11 from C.
    const C_MAJOR: u16 = 2773;

    fn settings(bars: u8, seed: u64) -> Settings {
        Settings {
            bars,
            seed,
            key: None,
            scale: None,
            octave: 60,
            duration: 1.0,
            rest: 0.0,
            arp: 0.0,
            chords: 4,
            tempo: None,
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

    #[test]
    fn the_same_seed_gives_the_same_notes() {
        let a = generate(&settings(4, 42), (0, C_MAJOR), &[], 4.0);
        let b = generate(&settings(4, 42), (0, C_MAJOR), &[], 4.0);
        assert_eq!(keys(&a), keys(&b));
        assert!(!a.is_empty());
    }

    #[test]
    fn every_note_is_in_the_scale_and_inside_the_register_window() {
        let events = generate(&settings(8, 7), (0, C_MAJOR), &[], 4.0);
        let pcs: Vec<i32> = offsets(C_MAJOR).iter().map(|o| o.rem_euclid(12)).collect();
        for key in keys(&events) {
            assert!((60..72).contains(&i32::from(key)), "key {key} outside C4..B4");
            assert!(pcs.contains(&i32::from(key % 12)), "key {key} not in C major");
        }
    }

    #[test]
    fn the_slots_fill_the_whole_length() {
        let events = generate(&settings(2, 3), (0, C_MAJOR), &[], 4.0);
        let end = events
            .iter()
            .map(|e| match e {
                Event::Note { pos, len_beats, .. } => pos + (*len_beats * f64::from(PPQ)) as u32,
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        // 2 bars of 4 beats at 128 ticks a beat.
        assert_eq!(end, 2 * 4 * PPQ);
    }

    #[test]
    fn chords_only_writes_no_single_melody_note() {
        let mut s = settings(2, 9);
        s.chords = 2;
        let events = generate(&s, (0, C_MAJOR), &[], 4.0);
        // Every onset is a triad (or a 7th): more notes than slots, all at slot starts.
        assert!(events.len() >= 3 * 2, "expected chords, got {}", events.len());
    }
}
