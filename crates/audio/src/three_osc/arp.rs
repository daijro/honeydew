//! 3x Osc Arpeggio and the key scheduler the instrument drives through [`Scheduler`]: keys go to the arp while
//! Direction ≠ Off, else through Echo. Tick-driven and allocation-free.

use super::echo::Echo;
use super::{NoteSpec, Params, Scheduler, Tick, VoiceEvent, gate_t, toggle};

/// Fixed-chord arps running at once (one per held key).
// ponytail: fixed pool; a key pressed while all are busy gets no arp.
pub const MAX_ARPS: usize = 32;
/// Arp notes take tags from here up (echo tags stay below, originals are their key).
pub const ARP_TAGS: u32 = 1 << 31;
/// Chord values (llround(90·v − 2)): −2 Auto, −1 Auto (sustain), 0 (none) = root only, k ≥ 1 = `CHORDS[k − 1]`.
pub const CHORD_AUTO: i32 = -2;
pub const CHORD_AUTO_SUSTAIN: i32 = -1;
/// Range --- raises the j-th note by 12·min(j, 5) st.
const RANGE_OFF_MAX_OCTAVE: usize = 5;

/// The 88 chord tables (Chord value k = 1..=88 is `CHORDS[k − 1]`): semitone offsets from the source note, root
/// included.
pub const CHORDS: [(&str, &[u8]); 88] = [
    ("Major", &[0, 4, 7]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("Majb5", &[0, 4, 6]),
    ("minor", &[0, 3, 7]),
    ("mb5", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("augsus4", &[0, 5, 8]),
    ("tri", &[0, 3, 6, 9]),
    ("6", &[0, 4, 7, 9]),
    ("6sus4", &[0, 5, 7, 9]),
    ("6add9", &[0, 4, 7, 9, 14]),
    ("m6", &[0, 3, 7, 9]),
    ("m6add9", &[0, 3, 7, 9, 14]),
    ("7", &[0, 4, 7, 10]),
    ("7sus4", &[0, 5, 7, 10]),
    ("7#5", &[0, 4, 8, 10]),
    ("7b5", &[0, 4, 6, 10]),
    ("7#9", &[0, 4, 7, 10, 15]),
    ("7b9", &[0, 4, 7, 10, 13]),
    ("7#5#9", &[0, 4, 8, 10, 15]),
    ("7#5b9", &[0, 4, 8, 10, 13]),
    ("7b5b9", &[0, 4, 6, 10, 13]),
    ("7add11", &[0, 4, 7, 10, 17]),
    ("7add13", &[0, 4, 7, 10, 21]),
    ("7#11", &[0, 4, 7, 10, 18]),
    ("Maj7", &[0, 4, 7, 11]),
    ("Maj7b5", &[0, 4, 6, 11]),
    ("Maj7#5", &[0, 4, 8, 11]),
    ("Maj7#11", &[0, 4, 7, 11, 18]),
    ("Maj7add13", &[0, 4, 7, 11, 21]),
    ("m7", &[0, 3, 7, 10]),
    ("m7b5", &[0, 3, 6, 10]),
    ("m7b9", &[0, 3, 7, 10, 13]),
    ("m7add11", &[0, 3, 7, 10, 17]),
    ("m7add13", &[0, 3, 7, 10, 21]),
    ("m-Maj7", &[0, 3, 7, 11]),
    ("m-Maj7add11", &[0, 3, 7, 11, 17]),
    ("m-Maj7add13", &[0, 3, 7, 11, 21]),
    ("9", &[0, 4, 7, 10, 14]),
    ("9sus4", &[0, 5, 7, 10, 14]),
    ("add9", &[0, 4, 7, 14]),
    ("9#5", &[0, 4, 8, 10, 14]),
    ("9b5", &[0, 4, 6, 10, 14]),
    ("9#11", &[0, 4, 7, 10, 14, 18]),
    ("9b13", &[0, 4, 7, 10, 14, 20]),
    ("Maj9", &[0, 4, 7, 11, 14]),
    ("Maj9sus4", &[0, 5, 7, 11, 14]),
    ("Maj9#5", &[0, 4, 8, 11, 14]),
    ("Maj9#11", &[0, 4, 7, 11, 14, 18]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("madd9", &[0, 3, 7, 14]),
    ("m9b5", &[0, 3, 6, 10, 14]),
    ("m9-Maj7", &[0, 3, 7, 11, 14]),
    ("11", &[0, 4, 7, 10, 14, 17]),
    ("11b9", &[0, 4, 7, 10, 13, 17]),
    ("Maj11", &[0, 4, 7, 11, 14, 17]),
    ("m11", &[0, 3, 7, 10, 14, 17]),
    ("m-Maj11", &[0, 3, 7, 11, 14, 17]),
    ("13", &[0, 4, 7, 10, 14, 21]),
    ("13#9", &[0, 4, 7, 10, 15, 21]),
    ("13b9", &[0, 4, 7, 10, 13, 21]),
    ("13b5b9", &[0, 4, 6, 10, 13, 21]),
    ("Maj13", &[0, 4, 7, 11, 14, 21]),
    ("m13", &[0, 3, 7, 10, 14, 21]),
    ("m-Maj13", &[0, 3, 7, 11, 14, 21]),
    ("Major scale", &[0, 2, 4, 5, 7, 9, 11]),
    ("Major pentatonic scale", &[0, 2, 4, 7, 9]),
    ("Major bebop scale", &[0, 2, 4, 5, 7, 8, 9, 11]),
    ("Minor Harmonic scale", &[0, 2, 3, 5, 7, 8, 11]),
    ("Minor Melodic scale", &[0, 2, 3, 5, 7, 9, 11]),
    ("Minor pentatonic scale", &[0, 3, 5, 7, 10]),
    ("Minor Natural scale", &[0, 2, 3, 5, 7, 8, 10]),
    ("Minor Neapolitan scale", &[0, 1, 3, 5, 7, 8, 11]),
    ("Minor Hungarian scale", &[0, 2, 3, 6, 7, 8, 11]),
    ("Whole tone scale", &[0, 2, 4, 6, 8, 10]),
    ("Diminished scale", &[0, 2, 3, 5, 6, 8, 9, 11]),
    ("Dominant bebop scale", &[0, 2, 4, 5, 7, 9, 10, 11]),
    ("Jap in sen scale", &[0, 1, 5, 7, 10]),
    ("Blues scale", &[0, 3, 5, 6, 7, 10]),
    ("Arabic scale", &[0, 1, 4, 5, 7, 8, 11]),
    ("Enigmatic scale", &[0, 1, 4, 6, 8, 10, 11]),
    ("Neapolitan scale", &[0, 1, 3, 5, 7, 9, 11]),
    ("Dorian scale", &[0, 2, 3, 5, 7, 9, 10]),
    ("Phrygian scale", &[0, 1, 3, 5, 7, 8, 10]),
    ("Lydian scale", &[0, 2, 4, 6, 7, 9, 11]),
    ("Mixolydian scale", &[0, 2, 4, 5, 7, 9, 10]),
    ("Locrian scale", &[0, 1, 3, 5, 6, 8, 10]),
];

/// The Arpeggio params (139 Direction … 145 Chord).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArpParams {
    /// 0 Off, 1 Up, 2 Down, 3 Bounce, 4 Sticky, 5 Random.
    pub direction: u8,
    /// Normalized; ≥ 1 holds.
    pub time: f32,
    pub gate: f32,
    /// 0 = ---, else 1..=6 octaves.
    pub range: u8,
    /// 1..=6 (--- = 1).
    pub repeat: u8,
    pub slide: bool,
    /// llround(90·v − 2): see [`CHORD_AUTO`].
    pub chord: i32,
}

/// The Chord value (p145): llround(90·v − 2), the product and the difference each rounded in f32; menu index = value
/// + 2. The arp and the menu text both use it.
pub fn chord_value(v: f32) -> i32 {
    (90.0 * v - 2.0).round() as i32
}

impl ArpParams {
    pub fn read(params: &Params) -> Self {
        let repeat = (params[143] * 5.0).round() as u8;
        Self {
            direction: (params[139] * 5.0).round() as u8,
            time: params[140],
            gate: params[141],
            range: (params[142] * 6.0).round() as u8,
            repeat: if repeat == 0 { 1 } else { repeat + 1 },
            slide: toggle(params[144]),
            chord: chord_value(params[145]),
        }
    }

    /// The step in (fractional) ticks by the Gate law: max(1 tick, t/192 beat) = max(1, 2t/D). `None` at
    /// Time = 100 % (hold). Steps accumulate on this exact grid, so an odd t does not drift.
    pub fn step_ticks(&self, tick: &Tick) -> Option<f64> {
        (self.time < 1.0)
            .then(|| (gate_t(self.time) as f64 * 384.0 / 192.0 / tick.d as f64).max(1.0))
    }
}

/// The sequence index of arp step `count` in a sequence of `n` (> 0) notes. Up in order; Down reversed; Bounce up
/// then down without repeating the ends; Sticky up then down repeating both ends; Random uniform per step, drawn
/// from `seed` and `count`.
pub fn direction_index(direction: u8, count: u64, n: usize, seed: u32) -> usize {
    let n = n as u64;
    (match direction {
        2 => n - 1 - count % n,
        3 if n < 2 => 0,
        3 => {
            let m = count % (2 * n - 2);
            if m < n { m } else { 2 * n - 2 - m }
        }
        4 => {
            let m = count % (2 * n);
            if m < n { m } else { 2 * n - 1 - m }
        }
        5 => splitmix(seed as u64 ^ splitmix(count)) % n,
        _ => count % n,
    }) as usize
}

/// SplitMix64's finalizer: a well-mixed hash for the Random direction.
fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Copy, Debug, Default)]
struct HeldKey {
    key: u8,
    velocity: f32,
    pan: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Runner {
    active: bool,
    /// Runs Auto over every held key; else a fixed chord on `root`.
    auto: bool,
    root: HeldKey,
    /// The next step's number.
    count: u64,
    /// The current step's exact grid position and the tick it fired on.
    step_at: f64,
    fired: u64,
    started: bool,
    /// This step's gate point has passed (Off sent, or Slide begun).
    gated: bool,
    sustaining: bool,
    tag: Option<u32>,
    seed: u32,
}

/// The 3x Osc key scheduler: Arpeggio while Direction ≠ Off, else Echo (arp notes never echo).
pub struct ArpEcho {
    echo: Echo,
    /// Held arp keys, sorted by key.
    held: [HeldKey; 128],
    n_held: usize,
    runners: [Runner; MAX_ARPS],
    next_tag: u32,
    next_seed: u32,
}

impl Default for ArpEcho {
    fn default() -> Self {
        Self {
            echo: Echo::default(),
            held: [HeldKey::default(); 128],
            n_held: 0,
            runners: [Runner::default(); MAX_ARPS],
            next_tag: ARP_TAGS,
            next_seed: 0x9E37_79B9,
        }
    }
}

impl Scheduler for ArpEcho {
    fn key_on(
        &mut self,
        key: u8,
        velocity: f32,
        pan: f32,
        params: &Params,
        tick: &Tick,
        emit: &mut dyn FnMut(VoiceEvent),
    ) {
        if key > 127 {
            return;
        }
        let p = ArpParams::read(params);
        if p.direction == 0 {
            self.echo.key_on(key, velocity, pan, params, tick, emit);
            return;
        }
        let held = HeldKey { key, velocity, pan };
        let pos = self.held[..self.n_held].partition_point(|h| h.key < key);
        if pos < self.n_held && self.held[pos].key == key {
            self.held[pos] = held;
        } else {
            self.held.copy_within(pos..self.n_held, pos + 1);
            self.held[pos] = held;
            self.n_held += 1;
        }
        let auto = p.chord < 0;
        self.stop(emit, |r| r.auto != auto); // a Chord switch since the last tick
        if auto && self.runners.iter().any(|r| r.active && r.auto) {
            return; // the one Auto arp now includes the key
        }
        self.stop(emit, |r| !r.auto && r.root.key == key); // a re-pressed key restarts its arp
        if let Some(r) = self.runners.iter_mut().find(|r| !r.active) {
            *r = Runner {
                active: true,
                auto,
                root: held,
                seed: self.next_seed,
                ..Runner::default()
            };
            self.next_seed = self.next_seed.wrapping_add(0x9E37_79B9);
        }
    }

    fn key_off(
        &mut self,
        key: u8,
        _params: &Params,
        tick: &Tick,
        emit: &mut dyn FnMut(VoiceEvent),
    ) {
        if key > 127 {
            return;
        }
        self.echo.key_off(key, tick, emit);
        let Some(pos) = self.held[..self.n_held].iter().position(|h| h.key == key) else {
            return;
        };
        self.held.copy_within(pos + 1..self.n_held, pos);
        self.n_held -= 1;
        let none_held = self.n_held == 0;
        self.stop(emit, |r| if r.auto { none_held } else { r.root.key == key });
    }

    fn tick(&mut self, params: &Params, tick: &Tick, emit: &mut dyn FnMut(VoiceEvent)) {
        self.echo.tick(tick, emit);
        let p = ArpParams::read(params);
        if p.direction == 0 {
            self.stop(emit, |_| true); // Direction Off with keys held: the arps stop
            return;
        }
        // A Chord switch between Auto and a fixed chord ends the arps of the other kind; the held
        // keys come back at the next note-on (Auto: all of them, in the one arp; fixed: the pressed key).
        self.stop(emit, |r| r.auto != (p.chord < 0));
        let step = p.step_ticks(tick);
        let now = tick.count;
        for i in 0..MAX_ARPS {
            let mut r = self.runners[i];
            if !r.active {
                continue;
            }
            if r.auto && p.chord == CHORD_AUTO_SUSTAIN && self.n_held == 1 {
                // Auto (sustain) with one held key just sustains it.
                if !r.sustaining {
                    r.silence(emit);
                    let k = self.held[0];
                    let note = NoteSpec {
                        key: k.key,
                        velocity: k.velocity,
                        pan: k.pan,
                        ..NoteSpec::default()
                    };
                    r = Runner {
                        sustaining: true,
                        count: 0,
                        tag: Some(self.on(note, emit)),
                        ..r
                    };
                }
                r.started = false;
                self.runners[i] = r;
                continue;
            }
            r.sustaining = false;
            // The gate point: Off, or with Slide the glide toward the next step's note.
            if let (true, false, Some(s), Some(tag)) = (r.started, r.gated, step, r.tag) {
                let gate = ((p.gate as f64 * s).round() as u64).max(1);
                if now >= r.fired + gate {
                    r.gated = true;
                    if !p.slide {
                        r.silence(emit);
                    } else if let Some(note) = self.note(&r, &p) {
                        let ticks = ((r.step_at + s).ceil() - now as f64).max(0.0) as u32;
                        emit(VoiceEvent::Slide { tag, note, ticks });
                    }
                }
            }
            // The step.
            let next = match step {
                _ if !r.started => Some(now as f64),
                // On the grid; after a Time change that leaves it a step behind, restart it here.
                Some(s) if now as f64 >= r.step_at + s => {
                    Some(if now as f64 >= r.step_at + 2.0 * s {
                        now as f64
                    } else {
                        r.step_at + s
                    })
                }
                _ => None,
            };
            if let Some(at) = next
                && let Some(note) = self.note(&r, &p)
            {
                match r.tag {
                    Some(tag) if p.slide => {
                        if !r.gated {
                            emit(VoiceEvent::Slide {
                                tag,
                                note,
                                ticks: 0,
                            });
                        }
                    }
                    _ => {
                        r.silence(emit);
                        r.tag = Some(self.on(note, emit));
                    }
                }
                r.count += 1;
                r.gated = false;
                r.started = true;
                r.step_at = at;
                r.fired = now;
            }
            self.runners[i] = r;
        }
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

impl Runner {
    /// Ends the sounding note, if any.
    fn silence(&mut self, emit: &mut dyn FnMut(VoiceEvent)) {
        if let Some(tag) = self.tag.take() {
            emit(VoiceEvent::Off { tag });
        }
    }
}

impl ArpEcho {
    /// Ends the running arps that `which` picks.
    fn stop(&mut self, emit: &mut dyn FnMut(VoiceEvent), which: impl Fn(&Runner) -> bool) {
        for r in self.runners.iter_mut().filter(|r| r.active && which(r)) {
            r.silence(emit);
            r.active = false;
        }
    }

    /// Runner `r`'s note for its step `r.count`: an Auto arp walks the held keys, a fixed one its root + the chord
    /// table; Range r ≥ 1 repeats the set r times, copy i at +12·i, each copy Repeat times in a row; Range ---
    /// raises the j-th note by 12·min(j, 5).
    fn note(&self, r: &Runner, p: &ArpParams) -> Option<NoteSpec> {
        let set: &[u8] = if p.chord >= 1 {
            CHORDS[p.chord as usize - 1].1
        } else {
            &[0]
        };
        let m = if r.auto { self.n_held } else { set.len() };
        if m == 0 {
            return None;
        }
        let block = m * p.repeat as usize;
        let i = direction_index(
            p.direction,
            r.count,
            block * p.range.max(1) as usize,
            r.seed,
        );
        let j = i % m;
        let octave = if p.range == 0 {
            j.min(RANGE_OFF_MAX_OCTAVE)
        } else {
            i / block
        };
        let (base, offset) = if r.auto {
            (self.held[j], 0)
        } else {
            (r.root, set[j])
        };
        Some(NoteSpec {
            key: base.key,
            velocity: base.velocity,
            pan: base.pan,
            pitch: (offset as usize + 12 * octave) as f32,
            ..NoteSpec::default()
        })
    }

    /// Emits `note` under a fresh arp tag and returns the tag.
    fn on(&mut self, note: NoteSpec, emit: &mut dyn FnMut(VoiceEvent)) -> u32 {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1) | ARP_TAGS;
        emit(VoiceEvent::On { tag, note });
        tag
    }
}

#[cfg(test)]
mod tests {
    use super::super::env::tests::T120;
    use super::*;

    /// Arp on: Up, Time 0.576 (t = 48: 24-tick steps), Gate 50 %, Range 1, Repeat ---, Slide off, Chord `Major` (value 1).
    fn base() -> Params {
        let mut p = [0.5; 149];
        p[120] = 0.0; // Mono off
        p[131] = 0.0; // Echo Feedback 0
        p[137] = 0.0; // Ping Pong off
        p[138] = 0.0; // Fat Mode off
        p[139] = 0.2; // Up
        p[140] = 0.576;
        p[141] = 0.5;
        p[142] = 1.0 / 6.0;
        p[143] = 0.0;
        p[144] = 0.0;
        p[145] = 3.0 / 90.0;
        p
    }

    type Log = Vec<(u64, VoiceEvent)>;

    /// Runs `ticks` ticks; `keys` are (tick, key, down) presses applied at their tick before the scheduler's tick.
    fn play(params: &Params, keys: &[(u64, u8, bool)], ticks: u64) -> Log {
        play_switching(params, params, 0, keys, ticks)
    }

    /// `play` with `before` until tick `switch`, then `after`.
    fn play_switching(
        before: &Params,
        after: &Params,
        switch: u64,
        keys: &[(u64, u8, bool)],
        ticks: u64,
    ) -> Log {
        let mut s = ArpEcho::default();
        let mut log = Vec::new();
        for count in 0..ticks {
            let params = if count < switch { before } else { after };
            let tick = Tick { count, ..T120 };
            let mut emit = |e| log.push((count, e));
            for &(at, key, down) in keys {
                if at == count {
                    if down {
                        s.key_on(key, 0.9, 0.5, params, &tick, &mut emit);
                    } else {
                        s.key_off(key, params, &tick, &mut emit);
                    }
                }
            }
            s.tick(params, &tick, &mut emit);
        }
        log
    }

    /// The On events' (tick, key + pitch).
    fn ons(log: &Log) -> Vec<(u64, f32)> {
        log.iter()
            .filter_map(|(t, e)| match e {
                VoiceEvent::On { note, .. } => Some((*t, note.key as f32 + note.pitch)),
                _ => None,
            })
            .collect()
    }

    /// The tags of the Off events at tick `t`.
    fn offs_at(log: &Log, t: u64) -> Vec<u32> {
        (log.iter())
            .filter_map(|(at, e)| match e {
                VoiceEvent::Off { tag } if *at == t => Some(*tag),
                _ => None,
            })
            .collect()
    }

    /// The tag of the one On event at tick `t`.
    fn on_tag_at(log: &Log, t: u64) -> u32 {
        let tags: Vec<u32> = (log.iter())
            .filter_map(|(at, e)| match e {
                VoiceEvent::On { tag, .. } if *at == t => Some(*tag),
                _ => None,
            })
            .collect();
        assert_eq!(tags.len(), 1, "one On at tick {t}");
        tags[0]
    }

    fn pitches(log: &Log) -> Vec<f32> {
        ons(log).iter().map(|o| o.1 - 60.0).collect()
    }

    fn step() -> u64 {
        gate_t(0.576) as u64 / 2
    }

    #[test]
    fn steps_follow_the_gate_law_and_arp_gate() {
        let s = step();
        assert_eq!(s, 24);
        let log = play(&base(), &[(0, 60, true)], 5 * s);
        let ons = ons(&log);
        assert_eq!(
            ons.iter().map(|o| o.0).collect::<Vec<_>>(),
            [0, s, 2 * s, 3 * s, 4 * s]
        );
        // Arp Gate 50 %: each note ends max(1, round(Gate·step)) ticks in.
        for (t, e) in &log {
            if let VoiceEvent::On { tag, .. } = e {
                let off = log
                    .iter()
                    .find(|(_, o)| *o == VoiceEvent::Off { tag: *tag });
                if let Some((off_t, _)) = off {
                    assert_eq!(off_t - t, s / 2);
                }
            }
        }
        // Gate 0: still one tick.
        let mut p = base();
        p[141] = 0.0;
        let log = play(&p, &[(0, 60, true)], 3);
        assert_eq!(log[1], (1, VoiceEvent::Off { tag: ARP_TAGS }));
    }

    #[test]
    fn arp_order_per_direction() {
        let run = |dir: f32, steps: u64| {
            let mut p = base();
            p[139] = dir;
            pitches(&play(&p, &[(0, 60, true)], steps * step()))
        };
        assert_eq!(run(0.2, 7), [0.0, 4.0, 7.0, 0.0, 4.0, 7.0, 0.0]);
        assert_eq!(run(0.4, 7), [7.0, 4.0, 0.0, 7.0, 4.0, 0.0, 7.0]);
        assert_eq!(run(0.6, 9), [0.0, 4.0, 7.0, 4.0, 0.0, 4.0, 7.0, 4.0, 0.0]);
        assert_eq!(run(0.8, 9), [0.0, 4.0, 7.0, 7.0, 4.0, 0.0, 0.0, 4.0, 7.0]);
        let random = run(1.0, 600);
        for pitch in [0.0, 4.0, 7.0] {
            let n = random.iter().filter(|p| **p == pitch).count();
            assert!((150..250).contains(&n), "{pitch}: {n} of 600");
        }
        assert!(random.iter().all(|p| [0.0, 4.0, 7.0].contains(p)));
        assert_ne!(random[..20], run(0.2, 20)[..]);
    }

    #[test]
    fn direction_index_edges() {
        for d in 1..=5 {
            assert_eq!(direction_index(d, 7, 1, 3), 0);
        }
        assert_eq!(
            (0..6)
                .map(|c| direction_index(3, c, 2, 0))
                .collect::<Vec<_>>(),
            [0, 1, 0, 1, 0, 1]
        );
    }

    #[test]
    fn range_and_repeat() {
        let mut p = base();
        p[145] = 2.0 / 90.0; // (none): root only
        p[142] = 2.0 / 6.0; // Range 2
        p[143] = 0.2; // Repeat 2
        assert_eq!(
            pitches(&play(&p, &[(0, 60, true)], 8 * step())),
            [0.0, 0.0, 12.0, 12.0, 0.0, 0.0, 12.0, 12.0]
        );
        // Major, Range 2, Repeat 2: each octave copy of the set emitted twice in a row.
        p[145] = 3.0 / 90.0;
        assert_eq!(
            pitches(&play(&p, &[(0, 60, true)], 12 * step())),
            [
                0.0, 4.0, 7.0, 0.0, 4.0, 7.0, 12.0, 16.0, 19.0, 12.0, 16.0, 19.0
            ]
        );
        // Range ---: the j-th note raised 12·min(j, 5).
        p[142] = 0.0;
        p[143] = 0.0;
        p[145] = 69.0 / 90.0; // value 67: Major scale
        assert_eq!(
            pitches(&play(&p, &[(0, 60, true)], 7 * step())),
            [0.0, 14.0, 28.0, 41.0, 55.0, 69.0, 71.0]
        );
    }

    #[test]
    fn chord_values_and_tables() {
        let chord = |v: f32| {
            let mut p = base();
            p[145] = v;
            ArpParams::read(&p).chord
        };
        assert_eq!(chord(0.0), CHORD_AUTO);
        assert_eq!(chord(0.0111), CHORD_AUTO_SUSTAIN); // the default
        assert_eq!(chord(2.0 / 90.0), 0);
        assert_eq!(chord(1.0), 88);
        assert_eq!(CHORDS[0], ("Major", &[0, 4, 7][..]));
        assert_eq!(CHORDS[8], ("tri", &[0, 3, 6, 9][..]));
        assert_eq!(CHORDS[87], ("Locrian scale", &[0, 1, 3, 5, 6, 8, 10][..]));
        for (name, set) in CHORDS {
            assert_eq!(set[0], 0, "{name}");
            assert!(set.windows(2).all(|w| w[0] < w[1]), "{name}");
        }
        // A fixed chord plays root + offsets.
        let mut p = base();
        p[145] = 81.0 / 90.0; // 79: Jap in sen
        assert_eq!(
            pitches(&play(&p, &[(0, 60, true)], 5 * step())),
            [0.0, 1.0, 5.0, 7.0, 10.0]
        );
    }

    #[test]
    fn auto_runs_one_arp_over_the_held_keys_sorted() {
        let mut p = base();
        p[145] = 0.0; // Auto
        let s = step();
        let log = play(&p, &[(0, 67, true), (0, 60, true), (0, 64, true)], 6 * s);
        let keys: Vec<f32> = ons(&log).iter().map(|o| o.1).collect();
        assert_eq!(keys, [60.0, 64.0, 67.0, 60.0, 64.0, 67.0]);
        // Auto with one key repeats it; Auto (sustain) just sustains it.
        assert_eq!(ons(&play(&p, &[(0, 60, true)], 3 * s)).len(), 3);
        p[145] = 1.0 / 90.0;
        let log = play(&p, &[(0, 60, true), (3 * s, 64, true)], 6 * s);
        assert_eq!(ons(&log)[0], (0, 60.0));
        assert!(!log.iter().any(|(t, _)| *t > 0 && *t < 3 * s));
        // A second key starts the arp; the sustained note ends.
        assert_eq!(
            ons(&log)[1..],
            [(3 * s, 60.0), (4 * s, 64.0), (5 * s, 60.0)]
        );
        assert_eq!(log[1], (3 * s, VoiceEvent::Off { tag: ARP_TAGS }));
    }

    #[test]
    fn key_up_stops_that_keys_arp() {
        let s = step();
        let mut p = base();
        p[141] = 1.0; // Gate 100 %: the note sounds until the key is up
        let log = play(
            &p,
            &[(0, 60, true), (0, 72, true), (s + 3, 60, false)],
            4 * s,
        );
        let after: Vec<f32> = ons(&log).iter().filter(|o| o.0 > s).map(|o| o.1).collect();
        assert_eq!(after, [79.0, 72.0]); // 72's arp only
        // 60's second note ends at the key-up.
        let tag_60 = log
            .iter()
            .find_map(|(t, e)| match e {
                VoiceEvent::On { tag, note } if *t == s && note.key == 60 => Some(*tag),
                _ => None,
            })
            .unwrap();
        assert!(log.contains(&(s + 3, VoiceEvent::Off { tag: tag_60 })));
    }

    #[test]
    fn time_100_percent_holds() {
        let mut p = base();
        p[140] = 1.0;
        let log = play(&p, &[(0, 60, true), (500, 60, false)], 600);
        assert_eq!(ons(&log), [(0, 60.0)]);
        assert_eq!(log[1], (500, VoiceEvent::Off { tag: ARP_TAGS }));
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn slide_glides_over_the_rest_of_the_step_without_retrigger() {
        let s = step();
        let mut p = base();
        p[144] = 1.0;
        let log = play(&p, &[(0, 60, true)], 3 * s);
        assert_eq!(ons(&log), [(0, 60.0)]);
        let slides: Vec<(u64, u32, f32, u32)> = log
            .iter()
            .filter_map(|(t, e)| match e {
                VoiceEvent::Slide { tag, note, ticks } => Some((*t, *tag, note.pitch, *ticks)),
                _ => None,
            })
            .collect();
        // At each gate point (Gate 50 %) toward the next step's note, arriving at the step.
        assert_eq!(
            slides,
            [
                (s / 2, ARP_TAGS, 4.0, (s / 2) as u32),
                (s + s / 2, ARP_TAGS, 7.0, (s / 2) as u32),
                (2 * s + s / 2, ARP_TAGS, 0.0, (s / 2) as u32)
            ]
        );
        assert!(!log.iter().any(|(_, e)| matches!(e, VoiceEvent::Off { .. })));
    }

    #[test]
    fn keys_pass_through_or_echo_while_the_arp_is_off() {
        let mut p = base();
        p[139] = 0.0;
        let log = play(&p, &[(0, 60, true), (10, 60, false)], 100);
        assert_eq!(
            log,
            [
                (
                    0,
                    VoiceEvent::On {
                        tag: 60,
                        note: NoteSpec {
                            key: 60,
                            velocity: 0.9,
                            pan: 0.5,
                            ..NoteSpec::default()
                        }
                    }
                ),
                (10, VoiceEvent::Off { tag: 60 })
            ]
        );
        // Echo on: the echoes come through the scheduler.
        p[131] = 0.4;
        p[130] = 0.0; // 1 echo
        let log = play(&p, &[(0, 60, true), (10, 60, false)], 100);
        assert_eq!(ons(&log), [(0, 60.0), (96, 60.0)]); // Time 0.5: 1 beat
        // Arp on: arp notes never echo.
        p[139] = 0.2;
        let log = play(&p, &[(0, 60, true), (10, 60, false)], 200);
        assert_eq!(ons(&log), [(0, 60.0)]);
    }

    #[test]
    fn reset_forgets_everything() {
        let mut s = ArpEcho::default();
        let mut log = Vec::new();
        s.key_on(60, 1.0, 0.5, &base(), &T120, &mut |e| log.push(e));
        s.reset();
        for count in 0..100 {
            s.tick(&base(), &Tick { count, ..T120 }, &mut |e| log.push(e));
        }
        assert!(log.is_empty());
    }

    #[test]
    fn steps_stay_on_the_grid_at_odd_t() {
        // Time 0.5: t = 37, a step of 37/192 beat = 18.5 ticks; step k fires on the first tick at or after 18.5·k.
        let mut p = base();
        p[140] = 0.5;
        assert_eq!(gate_t(0.5), 37);
        let log = play(&p, &[(0, 60, true)], 300);
        let ticks: Vec<u64> = ons(&log).iter().map(|o| o.0).collect();
        let grid: Vec<u64> = (0..=16).map(|k| (18.5 * k as f64).ceil() as u64).collect();
        assert_eq!(ticks[..17], grid[..]);
        // 16 steps = 16·37/192 beat exactly: tick 296, not 16·19 = 304.
        assert_eq!(ticks[16], 296);
    }

    #[test]
    fn direction_off_stops_running_arps() {
        let s = step();
        let mut arp = ArpEcho::default();
        let mut log = Vec::new();
        let mut p = base();
        for count in 0..4 * s {
            let tick = Tick { count, ..T120 };
            let mut emit = |e| log.push((count, e));
            if count == 0 {
                arp.key_on(60, 0.9, 0.5, &p, &tick, &mut emit);
            }
            if count == s + 3 {
                p[139] = 0.0;
                p[141] = 1.0; // the note would still be sounding
            }
            arp.tick(&p, &tick, &mut emit);
        }
        assert_eq!(ons(&log).len(), 2);
        // Exactly one Off at the switch, for the note sounding, and nothing after it.
        assert_eq!(offs_at(&log, s + 3), [on_tag_at(&log, s)]);
        assert_eq!(log.last().map(|e| e.0), Some(s + 3));
    }

    #[test]
    fn re_pressing_a_fixed_chord_key_restarts_its_arp() {
        let s = step();
        let mut p = base();
        p[141] = 1.0;
        let log = play(&p, &[(0, 60, true), (s + 3, 60, true)], 2 * s + 10);
        assert_eq!(
            pitches(&log),
            [0.0, 4.0, 0.0, 4.0] // steps 0, 1, then from step 0 again at the re-press
        );
        assert_eq!(ons(&log)[2].0, s + 3);
        assert_eq!(offs_at(&log, s + 3), [on_tag_at(&log, s)]);
    }

    #[test]
    fn auto_stops_only_when_the_last_key_is_up() {
        let s = step();
        let mut p = base();
        p[145] = 0.0; // Auto
        p[141] = 1.0;
        let log = play(
            &p,
            &[
                (0, 60, true),
                (0, 64, true),
                (s + 3, 60, false),
                (3 * s + 3, 64, false),
            ],
            5 * s,
        );
        let ons = ons(&log);
        assert_eq!(
            ons.iter().map(|o| o.1).collect::<Vec<_>>(),
            [60.0, 64.0, 64.0, 64.0]
        );
        assert_eq!(ons.last().unwrap().0, 3 * s);
        let last = log.last().unwrap();
        assert_eq!(last.0, 3 * s + 3);
        assert!(matches!(last.1, VoiceEvent::Off { .. }));
    }

    #[test]
    fn range_reads_seven_options() {
        let mut p = base();
        p[142] = 0.75; // round(6·0.75) = 5 octaves (round(5·0.75) would be 4)
        assert_eq!(ArpParams::read(&p).range, 5);
        p[145] = 2.0 / 90.0; // (none)
        assert_eq!(
            pitches(&play(&p, &[(0, 60, true)], 5 * step())),
            [0.0, 12.0, 24.0, 36.0, 48.0]
        );
    }

    #[test]
    fn auto_sustain_restarts_the_arp_from_step_zero() {
        let s = step();
        let mut p = base();
        p[145] = 1.0 / 90.0; // Auto (sustain)
        let keys = [
            (0, 60, true),
            (0, 64, true),
            (2 * s + 3, 64, false),
            (4 * s + 3, 67, true),
        ];
        let log = play(&p, &keys, 6 * s);
        // 60 64 60, then 60 sustained alone, then the arp from its first step: 60 before 67.
        assert_eq!(
            ons(&log).iter().map(|o| (o.0, o.1)).collect::<Vec<_>>(),
            [
                (0, 60.0),
                (s, 64.0),
                (2 * s, 60.0),
                (2 * s + 3, 60.0),
                (4 * s + 3, 60.0),
                (5 * s + 3, 67.0)
            ]
        );
    }

    /// A Chord switch between Auto and a fixed chord releases the arps of the other kind at once;
    /// the held keys come back at the next note-on.
    #[test]
    fn switching_auto_to_a_fixed_chord_releases_the_arp_until_the_next_key() {
        let s = step();
        let mut auto = base();
        auto[145] = 0.0; // Auto
        auto[141] = 1.0; // Gate 100 %: a note sounds until something ends it
        let mut major = auto;
        major[145] = 3.0 / 90.0;
        let keys = [(0, 60, true), (0, 64, true), (2 * s, 67, true)];
        let log = play_switching(&auto, &major, s + 3, &keys, 4 * s);
        // Auto 60 64; at the switch its note ends and 60, 64 fall silent; 67 starts its own Major arp.
        assert_eq!(
            ons(&log),
            [(0, 60.0), (s, 64.0), (2 * s, 67.0), (3 * s, 71.0)]
        );
        assert_eq!(offs_at(&log, s + 3), [on_tag_at(&log, s)]);
        assert!(!log.iter().any(|(t, _)| *t > s + 3 && *t < 2 * s));
    }

    #[test]
    fn switching_a_fixed_chord_to_auto_releases_every_arp_until_the_next_key() {
        let s = step();
        let mut major = base();
        major[141] = 1.0;
        let mut auto = major;
        auto[145] = 1.0 / 90.0; // Auto (sustain)
        let keys = [(0, 60, true), (0, 72, true), (2 * s, 67, true)];
        let log = play_switching(&major, &auto, s + 3, &keys, 5 * s);
        // Each key's Major arp ends at the switch; 67 starts the one Auto arp over all three held keys.
        assert_eq!(
            ons(&log),
            [
                (0, 60.0),
                (0, 72.0),
                (s, 64.0),
                (s, 76.0),
                (2 * s, 60.0),
                (3 * s, 67.0),
                (4 * s, 72.0)
            ]
        );
        let offs = |t| {
            log.iter()
                .filter(|e| e.0 == t && matches!(e.1, VoiceEvent::Off { .. }))
                .count()
        };
        assert_eq!(offs(s + 3), 2);
        assert!(!log.iter().any(|(t, _)| *t > s + 3 && *t < 2 * s));
        // A switch between fixed chords keeps the arp running on the new table.
        let mut minor = major;
        minor[145] = 7.0 / 90.0; // value 5: minor
        let log = play_switching(&major, &minor, 3, &[(0, 60, true)], 3 * s);
        assert_eq!(pitches(&log), [0.0, 3.0, 7.0]);
    }

    #[test]
    fn a_key_pressed_after_a_chord_switch_finds_the_full_pool_freed_without_a_tick() {
        // 32 fixed arps fill the pool, the Chord switches to Auto, and a key arrives before the next tick: `key_on`
        // alone must end the fixed arps, or the pool is full and the key gets no arp.
        let mut major = base();
        major[141] = 1.0; // Gate 100 %: a note sounds until something ends it
        let mut auto = major;
        auto[145] = 0.0;
        let mut arp = ArpEcho::default();
        let mut log = Log::new();
        let first = Tick { count: 0, ..T120 };
        for key in 40..40 + MAX_ARPS as u8 {
            arp.key_on(key, 0.9, 0.5, &major, &first, &mut |e| log.push((0, e)));
        }
        arp.tick(&major, &first, &mut |e| log.push((0, e)));
        assert_eq!(ons(&log).len(), MAX_ARPS, "every key has its arp");
        let next = Tick { count: 1, ..T120 };
        let mut pressed = Log::new();
        arp.key_on(100, 0.9, 0.5, &auto, &next, &mut |e| pressed.push((1, e)));
        assert_eq!(offs_at(&pressed, 1).len(), MAX_ARPS, "key_on ended them");
        arp.tick(&auto, &next, &mut |e| log.push((1, e)));
        // The one Auto arp over the 33 held keys starts on the lowest at once.
        let started: Vec<_> = ons(&log).into_iter().filter(|o| o.0 == 1).collect();
        assert_eq!(started, [(1, 40.0)]);
    }
}
