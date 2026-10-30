use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::sequencer::{
    ArpMode, ArpNoteEvent, ArpSettings, Arpeggiator, ClockMark, InputNoteEvent, NoteKind,
    SequencerError, SwingRate,
};

#[derive(Clone, Debug, PartialEq)]
pub struct NoteEffectModule {
    pub instance_id: i32,
    pub type_id: u16,
    pub params: Vec<f32>,
    /// Automation as `(beat, index, value)`, in beat order: from `beat` on, param `index` reads `value`. A note reads
    /// the params in force where it starts, and its note-off follows its note-on; the Arpeggiator switches its
    /// settings at each change.
    pub changes: Vec<(f64, usize, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoteEffectRack {
    pub modules: Vec<NoteEffectModule>,
    /// Beats per bar, the bar of the Arpeggiator's bar retrigger (TDIV byte 1); 4 from `new`.
    pub bar_beats: f64,
    /// The engine's sample rate and processing block, the frame grid of Note Echo's timing and collisions (frames
    /// count from the song start); 48 kHz and 1024 frames from `new`.
    pub sample_rate: f64,
    pub block_frames: u64,
}

/// What a note effect did over a song, for its panel: it runs ahead of playback, so the engine looks its live
/// state up here by the transport's beat.
#[derive(Clone, Debug, PartialEq)]
pub enum NoteEffectTrace {
    /// An Arpeggiator's two step clocks, each in beat order.
    Arp {
        instance_id: i32,
        clocks: [Vec<ClockMark>; 2],
    },
    /// A Note Echo's spans (beats) from a played note's start until its last echo ends, merged and in order: while
    /// the transport is in one, echoes are pending.
    Echo {
        instance_id: i32,
        pending: Vec<(f64, f64)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteEffectError {
    UnknownType(u16),
    InvalidParameter,
    InvalidTime,
    InvalidEvent,
    UnpairedNote(i16),
    TooManyEvents,
    Sequencer(SequencerError),
}

impl From<SequencerError> for NoteEffectError {
    fn from(value: SequencerError) -> Self {
        Self::Sequencer(value)
    }
}

/// A note, or with velocity 0 a note-off alone at `end` (Note Echo's repeat whose note-on was dropped).
#[derive(Clone, Copy)]
struct Note {
    start: f64,
    end: f64,
    pitch: i16,
    velocity: u8,
    pan: f32,
}

impl NoteEffectRack {
    pub fn new(modules: Vec<NoteEffectModule>) -> Result<Self, NoteEffectError> {
        for module in &modules {
            let count = match module.type_id {
                47 => 16,
                48 => 3,
                49 => 5,
                50 => 16,
                51 => 31,
                id => return Err(NoteEffectError::UnknownType(id)),
            };
            let valid = |value: &f32| value.is_finite() && (0.0..=1.0).contains(value);
            let mut previous = f64::NEG_INFINITY;
            for &(beat, index, value) in &module.changes {
                if !beat.is_finite()
                    || beat < previous
                    || index >= module.params.len()
                    || !valid(&value)
                {
                    return Err(NoteEffectError::InvalidParameter);
                }
                previous = beat;
            }
            if module.params.len() < count || !module.params.iter().all(valid) {
                return Err(NoteEffectError::InvalidParameter);
            }
        }
        Ok(Self {
            modules,
            bar_beats: 4.0,
            sample_rate: 48_000.0,
            block_frames: 1024,
        })
    }

    pub fn process_events(
        &self,
        start_beat: f64,
        end_beat: f64,
        tempo_bpm: f64,
        input: &[InputNoteEvent],
    ) -> Result<Vec<InputNoteEvent>, NoteEffectError> {
        self.process_events_traced(start_beat, end_beat, tempo_bpm, input, &mut Vec::new())
    }

    /// [`Self::process_events`], adding each Arpeggiator's and Note Echo's [`NoteEffectTrace`] to `traces`.
    pub fn process_events_traced(
        &self,
        start_beat: f64,
        end_beat: f64,
        tempo_bpm: f64,
        input: &[InputNoteEvent],
        traces: &mut Vec<NoteEffectTrace>,
    ) -> Result<Vec<InputNoteEvent>, NoteEffectError> {
        if !start_beat.is_finite()
            || !end_beat.is_finite()
            || end_beat <= start_beat
            || !tempo_bpm.is_finite()
            || tempo_bpm <= 0.0
            || !self.sample_rate.is_finite()
            || self.sample_rate <= 0.0
            || self.block_frames == 0
        {
            return Err(NoteEffectError::InvalidTime);
        }
        let mut previous = start_beat;
        for event in input {
            if !event.beat.is_finite()
                || event.beat < previous
                || event.beat >= end_beat
                || !(0..=127).contains(&event.pitch)
                || !event.pan.is_finite()
                || !(0.0..=1.0).contains(&event.pan)
            {
                return Err(NoteEffectError::InvalidEvent);
            }
            previous = event.beat;
        }
        let mut notes = pair(input)?;
        const MAX_NOTES: usize = 100_000;
        // The cap is on what note effects make: with none, the notes pass as they came (a channel of a Black MIDI
        // song holds hundreds of thousands; the song's own play budget bounds them).
        if !self.modules.is_empty() && notes.len() > MAX_NOTES {
            return Err(NoteEffectError::TooManyEvents);
        }
        for module in &self.modules {
            // A note that starts while the module is off (p0) passes as it is.
            let mut generated = Vec::new();
            notes = if module.type_id == 51 {
                let mut held = Vec::new();
                for_each_note(module, notes, |_, note, current| {
                    if current.params[0] < 0.5 || note.velocity == 0 {
                        generated.push(note);
                    } else {
                        held.push(note);
                    }
                    Ok(())
                })?;
                let (events, clocks) = arpeggiate(
                    module,
                    &events_from_notes(held),
                    (start_beat, end_beat),
                    tempo_bpm,
                    self.bar_beats,
                )?;
                traces.push(NoteEffectTrace::Arp {
                    instance_id: module.instance_id,
                    clocks,
                });
                generated.extend(pair(&events)?);
                generated
            } else {
                // Where Enable is switched Off (an echo's repeats stop there).
                let offs: Vec<f64> = (module.changes.iter())
                    .filter(|&&(_, index, value)| index == 0 && value < 0.5)
                    .map(|change| change.0)
                    .collect();
                let mut pending = Vec::new();
                let clock = EchoClock {
                    sample_rate: self.sample_rate,
                    tempo: tempo_bpm,
                    block: self.block_frames,
                };
                let input = match module.type_id {
                    47 => InputKeys::new(&notes, &clock),
                    _ => InputKeys::default(),
                };
                for_each_note(module, notes, |index, note, current| {
                    let stop = offs.get(offs.partition_point(|&off| off <= note.start));
                    let stop = stop.copied().unwrap_or(f64::INFINITY);
                    let from = generated.len();
                    per_note(current, note, index, stop, &input, &clock, &mut generated);
                    if module.type_id == 47 {
                        // Its echoes: what it added after the played note's start.
                        let echoes = generated[from..].iter().filter(|n| n.start > note.start);
                        let last = echoes.map(|n| n.end).fold(note.start, f64::max);
                        if last > note.start {
                            pending.push((note.start, last));
                        }
                    }
                    if generated.len() > MAX_NOTES {
                        return Err(NoteEffectError::TooManyEvents);
                    }
                    Ok(())
                })?;
                if module.type_id == 47 {
                    traces.push(NoteEffectTrace::Echo {
                        instance_id: module.instance_id,
                        pending: merged(pending),
                    });
                }
                generated
            };
            if notes.len() > MAX_NOTES {
                return Err(NoteEffectError::TooManyEvents);
            }
        }
        Ok(events_from_notes(notes))
    }
}

/// What a note effect other than the Arpeggiator makes of `note`, the `index`th in start order: the note as it is while
/// the module is off (p0); Note Echo, against the input notes `input` on `clock`, drops its repeats from `stop` on.
/// A note-off alone (an echo whose note-on was dropped): Transpose and Scale move its key as they move every note's;
/// Echo and the Randomizer pass it unchanged.
#[allow(clippy::too_many_arguments)]
fn per_note(
    module: &NoteEffectModule,
    note: Note,
    index: usize,
    stop: f64,
    input: &InputKeys,
    clock: &EchoClock,
    output: &mut Vec<Note>,
) {
    match module.type_id {
        _ if module.params[0] < 0.5
            || (note.velocity == 0 && !matches!(module.type_id, 48 | 50)) =>
        {
            output.push(note)
        }
        47 => echo(module, note, index, stop, input, clock, output),
        48 => transpose(module, note, output),
        49 => randomize(module, note, index, output),
        50 => scale(module, note, output),
        _ => unreachable!(),
    }
}

/// `spans` sorted, with overlapping and touching ones joined.
fn merged(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// Calls `f` with each of `notes` in start order, its index in that order, and `module` with its automation applied up
/// to where the note starts.
fn for_each_note(
    module: &NoteEffectModule,
    mut notes: Vec<Note>,
    mut f: impl FnMut(usize, Note, &NoteEffectModule) -> Result<(), NoteEffectError>,
) -> Result<(), NoteEffectError> {
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut current = NoteEffectModule {
        instance_id: module.instance_id,
        type_id: module.type_id,
        params: module.params.clone(),
        changes: Vec::new(),
    };
    let mut changes = module.changes.iter().peekable();
    for (index, note) in notes.into_iter().enumerate() {
        while let Some(&(_, param, value)) = changes.next_if(|change| change.0 <= note.start) {
            current.params[param] = value;
        }
        f(index, note, &current)?;
    }
    Ok(())
}

/// `events`, in beat order, as notes in start order. A note is placed at its note-on, so notes that start on one beat
/// keep their note-ons' order (a clip's order, as live keys keep their press order), not the order they end in.
fn pair(events: &[InputNoteEvent]) -> Result<Vec<Note>, NoteEffectError> {
    // Per key, the indices of its notes still held.
    let mut held: BTreeMap<i16, VecDeque<usize>> = BTreeMap::new();
    let mut notes = Vec::new();
    for event in events {
        match event.kind {
            // A note-on at velocity 0 is a note-off.
            NoteKind::On if event.velocity > 0 => {
                held.entry(event.pitch).or_default().push_back(notes.len());
                notes.push(Note {
                    start: event.beat,
                    end: f64::NAN,
                    pitch: event.pitch,
                    velocity: event.velocity,
                    pan: event.pan,
                });
            }
            NoteKind::On | NoteKind::Off => {
                let index = held
                    .get_mut(&event.pitch)
                    .and_then(VecDeque::pop_front)
                    .ok_or(NoteEffectError::UnpairedNote(event.pitch))?;
                let note = &mut notes[index];
                if event.beat <= note.start {
                    return Err(NoteEffectError::InvalidEvent);
                }
                note.end = event.beat;
            }
        }
    }
    if let Some((&pitch, _)) = held.iter().find(|(_, notes)| !notes.is_empty()) {
        return Err(NoteEffectError::UnpairedNote(pitch));
    }
    Ok(notes)
}

fn events_from_notes(notes: Vec<Note>) -> Vec<InputNoteEvent> {
    let mut events = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        // Velocity 0 is a note-off alone (`Note`).
        if note.velocity > 0 {
            events
                .push(InputNoteEvent::on(note.start, note.pitch, note.velocity).with_pan(note.pan));
        }
        events.push(InputNoteEvent::off(note.end, note.pitch).with_pan(note.pan));
    }
    crate::sequencer::sort_note_events(&mut events);
    events
}

/// Octave and Semitone: the clamped range map round(f32(f32(n·v) + lo)), halves away from zero, in f32 like their
/// texts.
fn transpose(module: &NoteEffectModule, note: Note, output: &mut Vec<Note>) {
    let octave = (8.0 * module.params[1] - 4.0).round() as i16;
    let semitone = (24.0 * module.params[2] - 12.0).round() as i16;
    let pitch = note.pitch + octave * 12 + semitone;
    if (0..=127).contains(&pitch) {
        output.push(Note { pitch, ..note });
    }
}

/// Randomizer: Pitch, Velocity and Pan spread the note; Gate (p4) is the chance it plays at all (100 %: always, 0 %:
/// never), and a note it drops goes with its note-off (issue #31: a probability, which Gate's 100 % default fits).
fn randomize(module: &NoteEffectModule, note: Note, index: usize, output: &mut Vec<Note>) {
    let mut random = Random::new(module.instance_id, index);
    let range = (24.0 * module.params[1]).round() as i16;
    let pitch = note.pitch + random.integer(-range, range);
    let velocity = (f64::from(note.velocity)
        * (1.0 + random.signed() * f64::from(module.params[2])))
    .round()
    .clamp(1.0, 255.0) as u8;
    let plays = random.unit() < f64::from(module.params[4]);
    let pan = (f64::from(note.pan) + random.signed() * f64::from(module.params[3])).clamp(0.0, 1.0)
        as f32;
    if plays {
        output.push(Note {
            pitch: pitch.clamp(0, 127),
            velocity,
            pan,
            ..note
        });
    }
}

/// `Delay (sync)`'s items in beats.
const SYNC_ITEMS: [f32; 10] = [0.125, 0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 3.0, 4.0, 8.0];

/// Note Echo. The played note passes unchanged: no offset touches it and it is never
/// dropped. Its repeats k = 1…R, R = llroundf(32·v1), clamp at each step:
/// - key₁ = clamp(clamp(key + PO) + PD), keyₖ = clamp(keyₖ₋₁ + PD) on 0…127, PO = llroundf(48·v5 − 24) and
///   PD = llroundf(24·v8 − 12);
/// - vel₁ = clamp(clamp(vel + VO)·VD), velₖ = clamp(velₖ₋₁·VD) on 0…1, VO from Vel. Offset and VD = 1 + Vel. Drift;
/// - bipolar pan₁ = clamp(pan + PnO), panₖ = clamp(panₖ₋₁ + PnD) on ±1, PnO = 2·v12 − 1 and PnD = v13 − 0.5; Ping-Pong
///   (v14 > 0.5) negates the emitted pan of repeats 2, 4, ….
///
/// Timing is in frames on `clock`: repeat 1 is D frames after the note-on, or D after the next block start
/// when that falls in the note's own block; the gap after repeat k is gₖ = f32(TD^(k+1)·D), randomized by Delay Rand.
/// from the second gap on; repeat k + 1 is at tₖ + floor(gₖ) when that reaches the block after the one repeat k was
/// processed in, and otherwise is processed in that block before its start: its note-on is dropped, its note-off and
/// the chain go on. Lengths follow the played note's held length (the gap while it is held), then Lₖ₋₁,
/// scaled by Length Drift and clamped by the gaps. The chain stops after a repeat at velocity ≤ 1/127 or shorter than
/// a tick.
///
/// A repeat whose key `input` holds, or within a frame of an input note-on of its key, is not emitted, not even a
/// dropped one's lone note-off; one at velocity 0 (a note-off) neither (its byte is otherwise at least 1). Repeats from `stop` on, where Enable is
/// switched Off, are dropped, and the note-offs still pending there are flushed to it.
fn echo(
    module: &NoteEffectModule,
    note: Note,
    index: usize,
    stop: f64,
    input: &InputKeys,
    clock: &EchoClock,
    output: &mut Vec<Note>,
) {
    let p = &module.params;
    output.push(note);
    let repeats = (32.0 * p[1]).round() as u32;
    // b = fma_f32(7.96875, v², 0.03125) beats: p2 while Sync (p4) is Off; while it is On, the item nearest p3's b (the
    // first of a tie).
    let beats = |v: f32| 7.96875f32.mul_add(v * v, 0.03125);
    let beats = if p[4] > 0.5 {
        let b = beats(p[3]);
        let distance = |item: &f32| (item - b).abs();
        SYNC_ITEMS
            .into_iter()
            .min_by(|x, y| distance(x).total_cmp(&distance(y)))
            .unwrap()
    } else {
        beats(p[2])
    };
    let delay = clock.frames(beats);
    // The clamped maps as f32 sums, TD = 0.7 + 0.6·v9 and LD = v + 0.5 up to the middle, 2v above.
    let time_drift = f64::from((0.6 * p[9] + 0.7).clamp(0.7, 1.3));
    let length_drift = if p[10] <= 0.5 {
        p[10] + 0.5
    } else {
        2.0 * p[10]
    };
    let tick = (clock.sample_rate * 0.46875 / clock.tempo).trunc();
    let key_drift = (24.0 * p[8] - 12.0).round() as i16;
    let velocity_drift = 1.0 + velocity_fraction(p[7]);
    let pan_drift = p[13] - 0.5;
    let mut random = Random::new(module.instance_id, index);
    let mut key = (note.pitch + (48.0 * p[5] - 24.0).round() as i16).clamp(0, 127);
    let mut velocity = (f32::from(note.velocity) / 255.0 + velocity_fraction(p[6])).clamp(0.0, 1.0);
    let mut pan = (2.0 * note.pan - 1.0 + 2.0 * p[12] - 1.0).clamp(-1.0, 1.0);
    let (on, off, stop_frame) = (
        clock.frame(note.start),
        clock.frame(note.end),
        clock.frame(stop),
    );
    let block = clock.block;
    let mut t = on + delay as u64;
    if t < (on / block + 1) * block {
        t += block - on % block;
    }
    // The block repeat k is processed in, and whether its note-on is delivered there.
    let (mut processed, mut sounds) = (t / block, true);
    let (mut gap, mut previous) = (delay as f32, 0.0_f32);
    for k in 1..=repeats {
        if t >= stop_frame {
            return;
        }
        key = (key + key_drift).clamp(0, 127);
        velocity = (velocity * velocity_drift).clamp(0.0, 1.0);
        if k > 1 {
            pan = (pan + pan_drift).clamp(-1.0, 1.0);
        }
        // Only repeat 1 can find the played note held; later repeats always take Lₖ₋₁.
        let prev = match k {
            1 if off > t => gap,
            1 => (off - on) as f32,
            _ => previous,
        };
        let mut length = length_drift
            .mul_add(prev, 0.5)
            .trunc()
            .min(gap.trunc())
            .max(0.0);
        // The stop rules read Lₖ before the clamp to the next gap, which needs a next repeat.
        let last = k == repeats || velocity <= 1.0 / 127.0 || f64::from(length) < tick;
        let next = (!last).then(|| {
            let g = (power(time_drift, k + 1) * delay) as f32;
            if p[11] > 0.0 {
                (p[11].mul_add(random.signed() as f32, 1.0) * g).max(0.0)
            } else {
                g
            }
        });
        if let Some(g) = next {
            length = length.min(g.trunc().max(1.0) - 1.0);
        }
        // A zero length ends a quarter frame after the start, on the same engine frame.
        let quarter = if length == 0.0 { 0.25 } else { 0.0 };
        let end = clock.beat(t as f64 + f64::from(length) + quarter).min(stop);
        let emitted = if p[14] > 0.5 && k % 2 == 0 { -pan } else { pan };
        let pan = (emitted + 1.0) / 2.0;
        // Input notes are never removed, so a colliding repeat sends nothing, not even a lone note-off.
        let collides = input.collides(key, t);
        if !collides && !sounds {
            output.push(Note {
                start: end,
                end,
                pitch: key,
                velocity: 0,
                pan,
            });
        } else if !collides && velocity > 0.0 {
            output.push(Note {
                start: clock.beat(t as f64),
                end,
                pitch: key,
                velocity: (velocity * 255.0).round().max(1.0) as u8,
                pan,
            });
        }
        previous = length;
        let Some(g) = next else {
            return;
        };
        let following = (processed + 1) * block;
        sounds = t as f64 + f64::from(g) >= following as f64;
        if sounds {
            t += g.floor() as u64;
            processed = t / block;
        } else {
            // In the following block before its start.
            t += g.ceil() as u64;
            processed += 1;
        }
        gap = g;
    }
}

/// x^n by repeated squaring in double, as compiler-rt's `__powidf2` (the drift power).
fn power(mut x: f64, mut n: u32) -> f64 {
    let mut result = 1.0;
    loop {
        if n & 1 == 1 {
            result *= x;
        }
        n /= 2;
        if n == 0 {
            return result;
        }
        x *= x;
    }
}

/// The frame grid of Note Echo's timing: the engine's rate, the tempo and the processing block, frames counted from
/// the song start.
struct EchoClock {
    sample_rate: f64,
    tempo: f64,
    block: u64,
}

impl EchoClock {
    /// The frame the engine places a note at `beat` on (`ceil`, as its `beat_frame`).
    fn frame(&self, beat: f64) -> u64 {
        (beat * 60.0 * self.sample_rate / self.tempo)
            .ceil()
            .max(0.0) as u64
    }

    /// A beat the engine places on `frame`: half a frame early, so its `ceil` lands on it exactly.
    fn beat(&self, frame: f64) -> f64 {
        (frame - 0.5) * self.tempo / (60.0 * self.sample_rate)
    }

    /// D = trunc(b·(60·fs)/BPM) frames, in double from the f32 `beats`.
    fn frames(&self, beats: f32) -> f64 {
        (f64::from(beats) * (60.0 * self.sample_rate) / self.tempo).trunc()
    }
}

/// A Note Echo's input notes by key, on the engine's frame grid: their start frames in order, each with the latest end
/// frame so far, so a collision test is two binary searches.
#[derive(Default)]
struct InputKeys(BTreeMap<i16, Vec<(u64, u64)>>);

impl InputKeys {
    fn new(notes: &[Note], clock: &EchoClock) -> Self {
        let mut keys: BTreeMap<i16, Vec<(u64, u64)>> = BTreeMap::new();
        for note in notes.iter().filter(|note| note.velocity > 0) {
            let frames = (clock.frame(note.start), clock.frame(note.end));
            keys.entry(note.pitch).or_default().push(frames);
        }
        for list in keys.values_mut() {
            list.sort_unstable();
            let mut latest = 0;
            for (_, end) in list.iter_mut() {
                latest = latest.max(*end);
                *end = latest;
            }
        }
        Self(keys)
    }

    /// An input note of `key` is held at `frame`, or starts within ±1 frame of it.
    fn collides(&self, key: i16, frame: u64) -> bool {
        let Some(list) = self.0.get(&key) else {
            return false;
        };
        let near = list.partition_point(|&(start, _)| start + 1 < frame);
        let started = list.partition_point(|&(start, _)| start <= frame);
        list.get(near).is_some_and(|&(start, _)| start <= frame + 1)
            || (started > 0 && list[started - 1].1 > frame)
    }
}

/// A −90 … 0 … +100 % knob (`Vel. Offset`, `Vel. Drift`) as a fraction, −0.9 … 1.
fn velocity_fraction(value: f32) -> f32 {
    if value < 0.5 {
        -0.9 * (1.0 - 2.0 * value)
    } else {
        2.0 * value - 1.0
    }
}

/// The notes p4–p15 (C first) are the scale: Key and Scale only write them, which the engine does
/// for automation as the panel does for edits. A note outside snaps to the nearest note that is on, the lower one first,
/// while Snap (p3) is On, and is dropped while it is Off (or when no note is on). A note and Snap are On above 0.5,
/// so 0.5 is Off.
fn scale(module: &NoteEffectModule, note: Note, output: &mut Vec<Note>) {
    let params = &module.params;
    let allowed = |pitch: i16| params[4 + pitch.rem_euclid(12) as usize] > 0.5;
    if allowed(note.pitch) {
        output.push(note);
    } else if params[3] > 0.5 {
        for distance in 1..=127 {
            for pitch in [note.pitch - distance, note.pitch + distance] {
                if (0..=127).contains(&pitch) && allowed(pitch) {
                    output.push(Note { pitch, ..note });
                    return;
                }
            }
        }
    }
}

/// The Arpeggiator's settings for params `p` in a song of `bar_beats` beats per bar. The toggles Sync, Latch and the
/// Arp 1 / Arp 2 enables are On above 0.5.
fn arp_settings(p: &[f32], bar_beats: f64) -> Result<ArpSettings, NoteEffectError> {
    let mut settings = ArpSettings {
        sync: p[3] > 0.5,
        sync_rate: menu(p[1], 8),
        free_rate_ms: 50.0 + f64::from(p[2]) * 950.0,
        swing: f64::from(p[4]),
        // A two-item menu, round(v): unlike the toggles, 0.5 picks the second item.
        swing_rate: if p[5] >= 0.5 {
            SwingRate::Sixteenth
        } else {
            SwingRate::Eighth
        },
        latch: p[6] > 0.5,
        bar_length_beats: bar_beats,
        ..ArpSettings::default()
    };
    for engine in 0..2 {
        let base = if engine == 0 { 9 } else { 20 };
        let e = &mut settings.engines[engine];
        e.enabled = p[base + 10] > 0.5;
        e.mode = ArpMode::from_menu_index(menu(p[base + 1], 24) as u8)?;
        e.gate = ((10.2 + 90.0 * f64::from(p[base])) / 100.0).min(1.0);
        e.range_octaves = 1 + menu(p[base + 2], 4) as u8;
        e.octave_shift = menu(p[base + 3], 7) as i8 - 3;
        e.start_note = menu(p[base + 4], 17) as i8 - 8;
        e.rhythm = menu(p[base + 5], 22) as u8;
        e.rotation = f64::from(p[base + 6]);
        let repeats = menu(p[base + 8], 17);
        e.repeats = if repeats == 0 {
            None
        } else {
            Some(repeats as u8)
        };
        e.set_retrigger_menu_index(menu(p[base + 9], 12) as u8)?;
        e.rate_multiplier = p.get(31 + engine).map_or(6, |value| menu(*value, 15));
        e.velocity_adjustment = (f64::from(p[7 + engine]) - 0.5) * 2.0;
    }
    Ok(settings)
}

/// Runs `events` through the Arpeggiator in blocks split at its automation, each with the settings in force there.
/// Every note still held at the end (latched ones too) is released then.
fn arpeggiate(
    module: &NoteEffectModule,
    events: &[InputNoteEvent],
    (start_beat, end_beat): (f64, f64),
    tempo_bpm: f64,
    bar_beats: f64,
) -> Result<(Vec<InputNoteEvent>, [Vec<ClockMark>; 2]), NoteEffectError> {
    let mut params = module.params.clone();
    let mut blocks = vec![(start_beat, arp_settings(&params, bar_beats)?)];
    for &(beat, index, value) in &module.changes {
        params[index] = value;
        blocks.push((beat.max(start_beat), arp_settings(&params, bar_beats)?));
    }
    let mut max_interval = 0.0_f64;
    for (_, settings) in &blocks {
        let longer = settings.step_interval(0, tempo_bpm)?;
        max_interval = max_interval.max(longer.max(settings.step_interval(1, tempo_bpm)?));
    }
    let last = events.last().map_or(end_beat, |event| event.beat);
    let horizon = if blocks.iter().any(|(_, settings)| settings.latch) {
        end_beat.max(last)
    } else {
        end_beat.max(last + max_interval + 1.0)
    };
    let mut arp = Arpeggiator::with_random_seed(blocks[0].1, module.instance_id as u64);
    arp.trace();
    let mut output = Vec::new();
    // Per pitch: the keys down, and whether the arp holds it (an Off is ignored while latched), as the arp sees them.
    let (mut down, mut held) = ([0_u32; 128], [false; 128]);
    let mut latched = false;
    for (k, &(from, settings)) in blocks.iter().enumerate() {
        let to = blocks
            .get(k + 1)
            .map_or(horizon, |next| next.0)
            .min(horizon);
        if to <= from {
            continue;
        }
        arp.settings = settings;
        // Latch switched Off releases the latched notes no longer held, there.
        let mut input = Vec::new();
        if latched && !settings.latch {
            for pitch in 0..128 {
                if held[pitch] && down[pitch] == 0 {
                    held[pitch] = false;
                    input.push(InputNoteEvent::off(from, pitch as i16));
                }
            }
        }
        let first = events.partition_point(|event| event.beat < from);
        let past = events.partition_point(|event| event.beat < to);
        for event in &events[first..past] {
            let pitch = event.pitch as usize;
            if event.kind == NoteKind::On {
                (down[pitch], held[pitch]) = (down[pitch] + 1, true);
            } else {
                down[pitch] = down[pitch].saturating_sub(1);
                held[pitch] &= settings.latch;
            }
        }
        input.extend_from_slice(&events[first..past]);
        output.extend(arp.process_block(from, to, tempo_bpm, &input)?);
        latched = settings.latch;
    }
    arp.settings.latch = false;
    let release = events
        .iter()
        .filter(|event| event.kind == NoteKind::On)
        .map(|event| event.pitch)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|pitch| InputNoteEvent::off(horizon, pitch))
        .collect::<Vec<_>>();
    output.extend(
        arp.process_block(horizon, horizon + max_interval + 1.0, tempo_bpm, &release)?
            .into_iter()
            .filter(|event| event.kind == NoteKind::Off),
    );
    let mut result: Vec<_> = output
        .into_iter()
        .filter(|event| (0..=127).contains(&event.pitch))
        .map(|event| InputNoteEvent {
            beat: event.beat,
            kind: event.kind,
            pitch: event.pitch,
            velocity: event.velocity,
            pan: event.pan,
        })
        .collect();
    result.sort_by(|a, b| {
        a.beat
            .total_cmp(&b.beat)
            .then_with(|| (a.kind != NoteKind::Off).cmp(&(b.kind != NoteKind::Off)))
    });
    // In beat order (a stable sort keeps the order of marks on one beat).
    let clocks = [0, 1].map(|engine| {
        let mut clock = arp.clock(engine).to_vec();
        clock.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        clock
    });
    Ok((result, clocks))
}

/// A note on or off the live chain sends. `level` is the number of note effects it
/// has passed: 0 is the key as played. On a notes lane (`notes_lane`) it goes to the instruments at that rack
/// position, as a sequenced note does; off one, only the copy past every note effect is sent, for every instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveNoteEvent {
    pub level: usize,
    pub notes_lane: bool,
    pub on: bool,
    pub key: u8,
    /// a/255, as for a sequenced note.
    pub velocity: f32,
    pub pan: f32,
    /// A note-off for a note an edit's chain took over: the host's instrument rows its note-on went to (bit r = row r,
    /// bit [`PADS`] the pads), whatever its level now reaches ([`LiveNoteChain::take_over`]).
    pub route: Option<u128>,
}

/// [`LiveNoteEvent::route`]'s bit for the pads; instrument rows from it on have none.
pub const PADS: usize = 127;

/// A note in the live chain. Times are live-clock frames; an end not known yet (a key still down) is infinite.
#[derive(Clone, Copy)]
struct LiveNote {
    id: u32,
    /// The note whose length this one follows, by factor `scale` (0: none).
    parent: u32,
    scale: f64,
    level: usize,
    notes_lane: bool,
    start: f64,
    end: f64,
    key: i16,
    velocity: u8,
    pan: f32,
    /// Its note-on went out.
    sent: bool,
    /// The stage it went into once sent (`level` until a [`LiveNoteChain::take_over`] moves that stage, while `level`
    /// stays where its note-on went out, for its note-off).
    into: usize,
    /// Its note-on went into the Arpeggiator of stage `into`, so its note-off goes there too.
    fed: bool,
    /// An echo repeat of stage `gate`: dropped if that Note Echo is off when it falls due, or if a note of its key at
    /// that stage's input sounds or starts within a frame of it.
    gate: Option<usize>,
    /// Made by the Arpeggiator of this stage, which sends its note-off later.
    from_arp: Option<usize>,
    /// Where its note-off goes, as it was sent before an edit's chain took it over ([`LiveNoteEvent::route`]).
    route: Option<u128>,
    /// Its Note Echo repeats are still to be made, with this random index, once the first falls due (frame
    /// `echo_at`) or its key lifts: either tells whether the key is held at repeat 1, which sets the lengths.
    echo: Option<usize>,
    echo_at: f64,
    done: bool,
}

/// A note effect as the live chain runs it: its params as they stand at song beat `beat` (`applied` of its
/// `changes`), and its count of notes so far (the random draws' index).
struct LiveStage {
    module: NoteEffectModule,
    initial: Vec<f32>,
    applied: usize,
    beat: f64,
    count: usize,
    arp: Option<LiveArp>,
}

/// An Arpeggiator in the live chain: the beat it has run to, its inputs and outputs, and per pitch the keys down and
/// whether it holds the note (latched ones too), as [`arpeggiate`] tracks them.
struct LiveArp {
    arp: Arpeggiator,
    last: f64,
    input: Vec<InputNoteEvent>,
    output: Vec<ArpNoteEvent>,
    down: [u32; 128],
    held: [bool; 128],
    notes_lane: bool,
}

/// The note effects of a channel run in real time on its live keys:
/// each live note passes them top to bottom as a sequenced note does in [`NoteEffectRack::process_events`], with the
/// params in force at the song position. A key's length is known only when it lifts, so the notes made from it end
/// then: an echo repeat that would end earlier ends at the lift. Its clock is the
/// engine's frames rendered, so it runs while the transport is stopped; the Arpeggiator's bar counts from that
/// clock. Nothing here allocates once built, up to `capacity` notes in flight (more are dropped).
pub struct LiveNoteChain {
    stages: Vec<LiveStage>,
    notes: Vec<LiveNote>,
    scratch: Vec<Note>,
    /// Note Echo's frame grid on the live clock.
    clock: EchoClock,
    frames_per_beat: f64,
    /// The live-clock frame of the Arpeggiators' beat 0: a live tempo moves it so their beat goes on where it stood.
    origin: f64,
    tempo_bpm: f64,
    bar_beats: f64,
    next_id: u32,
}

/// Note Echo's processing block on the live clock: the engine's.
const LIVE_BLOCK: u64 = 1024;

/// The frame a live-clock time falls on: the first whose start reaches it.
fn frame_of(time: f64) -> u64 {
    time.ceil().max(0.0) as u64
}

impl LiveNoteChain {
    /// The chain of `modules` (rack order, as for [`NoteEffectRack::new`], whose checks apply) in a song of
    /// `bar_beats` beats per bar at `tempo_bpm`, at `sample_rate`.
    pub fn new(
        modules: Vec<NoteEffectModule>,
        bar_beats: f64,
        sample_rate: f64,
        tempo_bpm: f64,
        capacity: usize,
    ) -> Result<Self, NoteEffectError> {
        let rate = sample_rate * 60.0 / tempo_bpm;
        if !rate.is_finite() || rate <= 0.0 || !bar_beats.is_finite() || bar_beats <= 0.0 {
            return Err(NoteEffectError::InvalidTime);
        }
        let mut stages = Vec::with_capacity(modules.len());
        for module in NoteEffectRack::new(modules)?.modules {
            let arp = if module.type_id == 51 {
                let settings = arp_settings(&module.params, bar_beats)?;
                Some(LiveArp {
                    arp: Arpeggiator::with_random_seed(settings, module.instance_id as u64),
                    last: 0.0,
                    input: Vec::with_capacity(256),
                    output: Vec::with_capacity(4096),
                    down: [0; 128],
                    held: [false; 128],
                    notes_lane: true,
                })
            } else {
                None
            };
            stages.push(LiveStage {
                initial: module.params.clone(),
                module,
                applied: 0,
                beat: f64::NEG_INFINITY,
                count: 0,
                arp,
            });
        }
        Ok(Self {
            stages,
            notes: Vec::with_capacity(capacity),
            scratch: Vec::with_capacity(64),
            clock: EchoClock {
                sample_rate,
                tempo: tempo_bpm,
                block: LIVE_BLOCK,
            },
            frames_per_beat: rate,
            origin: 0.0,
            tempo_bpm,
            bar_beats,
            next_id: 0,
        })
    }

    /// Key `key` goes down at frame `now`, with `velocity` in 0..=1 (stored as a byte, as a sequenced note's) and the
    /// keyboard's centre pan. `notes_lane`: the channel's current lane is a notes lane.
    pub fn note_on(&mut self, now: u64, key: u8, velocity: f32, notes_lane: bool) {
        let velocity = (velocity.clamp(0.0, 1.0) * 255.0).round().max(1.0) as u8;
        self.add(LiveNote {
            id: 0,
            parent: 0,
            scale: 1.0,
            level: 0,
            notes_lane,
            start: now as f64,
            end: f64::INFINITY,
            key: i16::from(key.min(127)),
            velocity,
            pan: 0.5,
            sent: false,
            into: 0,
            fed: false,
            gate: None,
            from_arp: None,
            route: None,
            echo: None,
            echo_at: f64::INFINITY,
            done: false,
        });
    }

    /// Key `key` lifts at frame `now`: the earliest key `key` still down ends, and the notes made from it with it.
    pub fn note_off(&mut self, now: u64, key: u8) {
        let held = (self.notes.iter().enumerate())
            .filter(|(_, n)| n.level == 0 && n.parent == 0 && !n.done && n.end.is_infinite())
            .filter(|(_, n)| n.key == i16::from(key))
            .min_by(|(_, a), (_, b)| a.start.total_cmp(&b.start));
        if let Some((index, _)) = held {
            end_note(&mut self.notes, index, now as f64, now as f64);
        }
    }

    /// The live tempo from frame `now` on (a tempo drag): the
    /// Arpeggiators' beat goes on where it stood at `now` and their steps follow the new beat (a free rate keeps its
    /// milliseconds); Note Echo times what it makes from then on at it, and a
    /// series still to be made falls due at the new tempo (its first repeat's frame scaled about the note's start).
    /// Notes already timed keep their frames. Allocates nothing.
    pub fn set_tempo(&mut self, now: u64, tempo_bpm: f64) {
        let rate = self.clock.sample_rate * 60.0 / tempo_bpm;
        if !rate.is_finite() || rate <= 0.0 || tempo_bpm == self.tempo_bpm {
            return;
        }
        let (beat, scale) = (self.arp_beat(now as f64), rate / self.frames_per_beat);
        for note in self.notes.iter_mut().filter(|note| note.echo.is_some()) {
            note.echo_at = note.start + (note.echo_at - note.start) * scale;
        }
        (self.frames_per_beat, self.tempo_bpm, self.clock.tempo) = (rate, tempo_bpm, tempo_bpm);
        self.origin = now as f64 - beat * rate;
    }

    /// Takes over `running`, the chain this one, built for an edited song, replaces, at frame `now`
    /// (moved modules keep their state, an added one starts empty,
    /// the instrument's voices and the other modules are untouched). Each note effect both hold (type and instance)
    /// keeps its state (its count of notes, an Arpeggiator's keys, latch and steps, Note Echo's series to come) in this
    /// one's rack order, with this one's params and automation, and the notes in flight go on at this one's tempo: one
    /// still to start goes in below the note effect that made it, one already sent ends where its note-on went out:
    /// `reroute(route, level, notes_lane)` gives that place as this engine's rows (its earlier route, else its level
    /// and lane in `running`'s), whatever moved, came or went around it. A removed note effect is destroyed: its notes
    /// still to start are dropped, an Arpeggiator's sounding ones end now (as Note Echo flushes its pending
    /// note-offs when switched off), and Note Echo makes no more repeats. `running` keeps the chain built here.
    /// Allocates nothing.
    pub fn take_over(
        &mut self,
        running: &mut LiveNoteChain,
        now: u64,
        mut reroute: impl FnMut(Option<u128>, usize, bool) -> u128,
    ) {
        let effect = |stage: &LiveStage| (stage.module.type_id, stage.module.instance_id);
        let (stages, old) = (&self.stages, &running.stages);
        // Where stage `q` of the running order is in this one (None: removed); past the last stays past the last.
        let to = |q: usize| match old.get(q) {
            Some(stage) => stages.iter().position(|s| effect(s) == effect(stage)),
            None => Some(stages.len()),
        };
        for index in 0..running.notes.len() {
            let note = &mut running.notes[index];
            if note.sent {
                if note.route.is_some() || note.notes_lane || note.level == old.len() {
                    note.route = Some(reroute(note.route, note.level, note.notes_lane));
                } else {
                    note.route = Some(0);
                }
                note.into = to(note.into).unwrap_or(usize::MAX);
                if note.into == usize::MAX {
                    (note.echo, note.echo_at) = (None, f64::INFINITY);
                }
            } else if note.level > 0 {
                match to(note.level - 1) {
                    Some(at) => (note.level, note.into) = (at + 1, at + 1),
                    None => note.done = true,
                }
            }
            note.gate = note.gate.and_then(to);
            let made = note.from_arp;
            note.from_arp = made.and_then(to);
            if made.is_some() && note.from_arp.is_none() && note.sent && note.end.is_infinite() {
                end_note(&mut running.notes, index, now as f64, now as f64);
            }
        }
        for stage in &mut self.stages {
            if let Some(theirs) = running
                .stages
                .iter_mut()
                .find(|r| effect(r) == effect(stage))
            {
                std::mem::swap(&mut stage.count, &mut theirs.count);
                std::mem::swap(&mut stage.arp, &mut theirs.arp);
            }
            // Its params again from this song's, so an Arpeggiator takes its settings (Latch switched Off releases).
            stage.beat = f64::INFINITY;
        }
        let tempo = self.tempo_bpm;
        // Into the room built for this chain's note effects (a Note Echo's), unless the notes in flight need more.
        if self.notes.capacity() >= running.notes.len() {
            self.notes.extend_from_slice(&running.notes);
        } else {
            std::mem::swap(&mut self.notes, &mut running.notes);
        }
        self.next_id = running.next_id;
        (self.origin, self.frames_per_beat) = (running.origin, running.frames_per_beat);
        (self.tempo_bpm, self.clock.tempo) = (running.tempo_bpm, running.clock.tempo);
        self.set_tempo(now, tempo);
    }

    /// The live keys down at its input (bit k = MIDI key k), which [`Self::note_off`] would end.
    pub fn keys_down(&self) -> u128 {
        (self.notes.iter())
            .filter(|n| n.level == 0 && n.parent == 0 && !n.done && n.end.is_infinite())
            .fold(0, |keys, n| keys | 1_u128 << n.key.clamp(0, 127))
    }

    /// Forgets every note in flight and resets the Arpeggiators: all notes off, a seek.
    pub fn clear(&mut self) {
        self.notes.clear();
        self.clear_sent();
    }

    /// [`Self::clear`], but the keys pressed and not yet sent stay queued: they go out at their frame, after it (Play
    /// start: the key whose press starts playback keeps sounding).
    pub fn clear_sent(&mut self) {
        self.notes.retain(|note| note.level == 0 && !note.sent);
        for arp in self
            .stages
            .iter_mut()
            .filter_map(|stage| stage.arp.as_mut())
        {
            arp.arp.reset();
            arp.input.clear();
            (arp.down, arp.held) = ([0; 128], [false; 128]);
        }
    }

    /// Appends to `out` every note on and off due by frame `now` (offs first), with the params in force at song beat
    /// `song_beat`.
    pub fn due(&mut self, now: u64, song_beat: f64, out: &mut Vec<LiveNoteEvent>) {
        self.sync(now, song_beat);
        loop {
            while self.pass(now, out) {}
            // The first Arpeggiator still to run to `now`, once every note above it is through.
            let end = self.arp_beat((now + 1) as f64);
            let next = (self.stages.iter())
                .position(|stage| stage.arp.as_ref().is_some_and(|arp| arp.last < end));
            match next {
                Some(stage) => self.run_arp(stage, now, end),
                None => break,
            }
        }
        self.notes.retain(|note| !note.done);
    }

    /// The next frame where [`Self::due`] has work, if any.
    pub fn next_due(&self) -> Option<u64> {
        let notes = (self.notes.iter()).filter_map(|note| match note.sent {
            false => Some(frame_of(note.start)),
            true => note.end.is_finite().then(|| frame_of(note.end)),
        });
        let echoes = (self.notes.iter())
            .filter(|note| note.echo.is_some() && !note.done)
            .map(|note| frame_of(note.echo_at));
        let notes = notes.chain(echoes);
        let arps = (self.stages.iter())
            .filter_map(|stage| stage.arp.as_ref()?.arp.next_event_beat(self.tempo_bpm))
            .map(|beat| frame_of(self.arp_frame(beat)));
        notes.chain(arps).min()
    }

    /// The Arpeggiators' beat at live-clock time `frame`.
    fn arp_beat(&self, frame: f64) -> f64 {
        (frame - self.origin) / self.frames_per_beat
    }

    /// The live-clock time of the Arpeggiators' beat `beat`.
    fn arp_frame(&self, beat: f64) -> f64 {
        beat * self.frames_per_beat + self.origin
    }

    /// Each stage's params as they stand at `song_beat`; an Arpeggiator takes the new settings, and Latch switched
    /// Off releases the latched notes no longer held at `now` (as [`arpeggiate`] does).
    fn sync(&mut self, now: u64, song_beat: f64) {
        let beat = self.arp_beat(now as f64);
        for stage in &mut self.stages {
            let before = stage.applied;
            if song_beat < stage.beat {
                stage.module.params.copy_from_slice(&stage.initial);
                stage.applied = 0;
            }
            let changes = &stage.module.changes;
            while let Some(&(at, index, value)) = changes.get(stage.applied) {
                if at > song_beat {
                    break;
                }
                stage.module.params[index] = value;
                stage.applied += 1;
            }
            let moved = stage.applied != before || song_beat < stage.beat;
            stage.beat = song_beat;
            let Some(arp) = stage.arp.as_mut().filter(|_| moved) else {
                continue;
            };
            let Ok(settings) = arp_settings(&stage.module.params, self.bar_beats) else {
                continue;
            };
            if arp.arp.settings.latch && !settings.latch {
                for pitch in 0..128 {
                    if arp.held[pitch] && arp.down[pitch] == 0 {
                        arp.held[pitch] = false;
                        arp.input.push(InputNoteEvent::off(beat, pitch as i16));
                    }
                }
            }
            arp.arp.settings = settings;
        }
    }

    /// One pass over the notes in flight: the note-offs due, then the note-ons due, each sent and passed to the next
    /// note effect. Whether anything happened.
    fn pass(&mut self, now: u64, out: &mut Vec<LiveNoteEvent>) -> bool {
        let mut progress = false;
        for index in 0..self.notes.len() {
            let note = self.notes[index];
            if note.echo.is_some() && (frame_of(note.echo_at) <= now || note.end.is_finite()) {
                self.echo(index);
                progress = true;
            }
        }
        for index in 0..self.notes.len() {
            let note = self.notes[index];
            if note.done || !note.sent || note.end.is_infinite() || frame_of(note.end) > now {
                continue;
            }
            self.notes[index].done = true;
            progress = true;
            self.send(note, false, out);
            let beat = self.arp_beat(note.end);
            if let Some(arp) = self.stages.get_mut(note.into).and_then(|s| s.arp.as_mut())
                && note.fed
            {
                arp.input.push(InputNoteEvent::off(beat, note.key));
            }
        }
        // Notes added while passing (a repeat due now) are reached in the same pass.
        let mut index = 0;
        while index < self.notes.len() {
            let note = self.notes[index];
            index += 1;
            if note.done || note.sent || frame_of(note.start) > now {
                continue;
            }
            progress = true;
            if note.gate.is_some_and(|stage| {
                self.stages[stage].module.params[0] < 0.5 || self.collides(stage, note.key, now)
            }) {
                self.notes[index - 1].done = true;
                continue;
            }
            self.notes[index - 1].sent = true;
            // Velocity 0: a note-off alone, which goes on as one.
            if note.velocity > 0 {
                self.send(note, true, out);
            }
            self.pass_on(index - 1, now);
        }
        progress
    }

    /// `note` leaves stage `note.level`: sent unless it is off a notes lane and some note effect is still to come; a
    /// note-off of a note taken over goes where its note-on went (its route), if anywhere.
    fn send(&self, note: LiveNote, on: bool, out: &mut Vec<LiveNoteEvent>) {
        let route = note.route.filter(|_| !on);
        if route.map_or(note.notes_lane || note.level == self.stages.len(), |rows| {
            rows != 0
        }) {
            out.push(LiveNoteEvent {
                level: note.level,
                notes_lane: note.notes_lane,
                on,
                key: note.key as u8,
                velocity: f32::from(note.velocity) / 255.0,
                pan: note.pan,
                route,
            });
        }
    }

    /// Note `index`, just started, into the note effect below it: the Arpeggiator takes it as a key (while on); any
    /// other makes the notes [`per_note`] makes of it, each following its length.
    fn pass_on(&mut self, index: usize, now: u64) {
        let note = self.notes[index];
        let beat = self.arp_beat(note.start);
        let Some(stage) = self.stages.get_mut(note.level) else {
            return;
        };
        let on = stage.module.params[0] >= 0.5 && note.velocity > 0;
        if let Some(arp) = stage.arp.as_mut().filter(|_| on) {
            arp.input
                .push(InputNoteEvent::on(beat, note.key, note.velocity).with_pan(note.pan));
            arp.notes_lane = note.notes_lane;
            self.notes[index].fed = true;
            return;
        }
        stage.count += 1;
        let random = stage.count - 1;
        self.scratch.clear();
        let unit = Note {
            start: 0.0,
            end: 1.0,
            pitch: note.key,
            velocity: note.velocity,
            pan: note.pan,
        };
        let echoes = on && stage.module.type_id == 47;
        if stage.arp.is_some() || echoes {
            // Note Echo passes the played note at once; its repeats wait ([`Self::echo`]).
            self.scratch.push(unit);
        } else {
            let keys = InputKeys::default();
            per_note(
                &stage.module,
                unit,
                random,
                f64::INFINITY,
                &keys,
                &self.clock,
                &mut self.scratch,
            );
        }
        if echoes {
            // The first repeat as if the key were held there.
            let held = Note {
                start: self.clock_beat(note.start),
                end: f64::INFINITY,
                ..unit
            };
            let mut series = std::mem::take(&mut self.scratch);
            let first = series.len();
            let keys = InputKeys::default();
            let stage = &self.stages[note.level];
            echo(
                &stage.module,
                held,
                random,
                f64::INFINITY,
                &keys,
                &self.clock,
                &mut series,
            );
            let at = series[first + 1..]
                .iter()
                .map(|made| made.start)
                .min_by(f64::total_cmp);
            series.truncate(first);
            self.scratch = series;
            self.notes[index].echo = Some(random);
            self.notes[index].echo_at =
                at.map_or(f64::INFINITY, |beat| beat * self.frames_per_beat);
        }
        for k in 0..self.scratch.len() {
            let made = self.scratch[k];
            let start = note.start + made.start * self.frames_per_beat;
            let scale = made.end - made.start;
            let end = match note.end.is_finite() {
                true => (start + (note.end - note.start) * scale).max(now as f64),
                false => f64::INFINITY,
            };
            self.add(LiveNote {
                id: 0,
                parent: note.id,
                scale,
                level: note.level + 1,
                start,
                end,
                key: made.pitch,
                velocity: made.velocity,
                pan: made.pan,
                sent: false,
                into: note.level + 1,
                fed: false,
                gate: None,
                from_arp: None,
                route: None,
                echo: None,
                echo_at: f64::INFINITY,
                done: false,
                ..note
            });
        }
    }

    /// A live-clock time as the beat Note Echo reads: half a frame early, so its frame is the same.
    fn clock_beat(&self, time: f64) -> f64 {
        if time.is_finite() {
            self.clock.beat(frame_of(time) as f64)
        } else {
            f64::INFINITY
        }
    }

    /// Makes note `index`'s Note Echo repeats: its key's end is known, or the first
    /// repeat is due with the key still held, so the series is the prepared one. A repeat that loses its note-on
    /// keeps its note-off, a note-off alone (velocity 0). Each is checked as it falls due ([`Self::collides`]).
    fn echo(&mut self, index: usize) {
        let note = self.notes[index];
        let Some(random) = self.notes[index].echo.take() else {
            return;
        };
        self.notes[index].echo_at = f64::INFINITY;
        let played = Note {
            start: self.clock_beat(note.start),
            end: self.clock_beat(note.end),
            pitch: note.key,
            velocity: note.velocity,
            pan: note.pan,
        };
        self.scratch.clear();
        let keys = InputKeys::default();
        let stage = &self.stages[note.into];
        echo(
            &stage.module,
            played,
            random,
            f64::INFINITY,
            &keys,
            &self.clock,
            &mut self.scratch,
        );
        for k in 1..self.scratch.len() {
            let made = self.scratch[k];
            self.add(LiveNote {
                id: 0,
                parent: 0,
                scale: 1.0,
                level: note.into + 1,
                notes_lane: note.notes_lane,
                start: made.start * self.frames_per_beat,
                end: made.end * self.frames_per_beat,
                key: made.pitch,
                velocity: made.velocity,
                pan: made.pan,
                sent: false,
                into: note.into + 1,
                fed: false,
                gate: Some(note.into),
                from_arp: None,
                route: None,
                echo: None,
                echo_at: f64::INFINITY,
                done: false,
            });
        }
    }

    /// A note of `key` at stage `stage`'s input sounds at frame `now`, or started within a frame of it (the prepared
    /// echo's input test; a live key that starts a frame later is not known yet).
    fn collides(&self, stage: usize, key: i16, now: u64) -> bool {
        (self.notes.iter())
            .filter(|n| n.into == stage && n.key == key && n.sent && n.velocity > 0)
            .any(|n| {
                let start = frame_of(n.start);
                start <= now + 1
                    && (start + 1 >= now || n.end.is_infinite() || frame_of(n.end) > now)
            })
    }

    /// Runs the Arpeggiator of `stage` from where it stopped to beat `end` (the start of frame `now + 1`), with the
    /// keys it was given; its notes go on to the next stage.
    fn run_arp(&mut self, stage: usize, now: u64, end: f64) {
        let Some(arp) = self.stages[stage].arp.as_mut() else {
            return;
        };
        // In time order, a note-off before a note-on at the same beat; none before where it stopped.
        let input = &mut arp.input;
        for i in 1..input.len() {
            let mut j = i;
            let key = |e: &InputNoteEvent| (e.beat, e.kind == NoteKind::On);
            while j > 0
                && key(&input[j - 1])
                    .partial_cmp(&key(&input[j]))
                    .is_some_and(|o| o.is_gt())
            {
                input.swap(j - 1, j);
                j -= 1;
            }
        }
        for event in input.iter_mut() {
            event.beat = event.beat.max(arp.last);
            let pitch = event.pitch.clamp(0, 127) as usize;
            if event.kind == NoteKind::On {
                (arp.down[pitch], arp.held[pitch]) = (arp.down[pitch] + 1, true);
            } else {
                arp.down[pitch] = arp.down[pitch].saturating_sub(1);
                arp.held[pitch] &= arp.arp.settings.latch;
            }
        }
        arp.output.clear();
        let result =
            arp.arp
                .process_block_into(arp.last, end, self.tempo_bpm, &arp.input, &mut arp.output);
        arp.input.clear();
        arp.last = end;
        if result.is_err() {
            arp.arp.reset();
            return;
        }
        let notes_lane = arp.notes_lane;
        for k in 0..arp.output.len() {
            let Some(arp) = self.stages[stage].arp.as_ref() else {
                return;
            };
            let event = arp.output[k];
            if !(0..=127).contains(&event.pitch) {
                continue;
            }
            let at = self.arp_frame(event.beat);
            if event.kind == NoteKind::Off {
                let open = (self.notes.iter().enumerate())
                    .filter(|(_, n)| n.from_arp == Some(stage) && n.key == event.pitch)
                    .filter(|(_, n)| !n.done && n.end.is_infinite())
                    .min_by(|(_, a), (_, b)| a.start.total_cmp(&b.start));
                if let Some((index, _)) = open {
                    end_note(&mut self.notes, index, at, now as f64);
                }
                continue;
            }
            self.add(LiveNote {
                id: 0,
                parent: 0,
                scale: 1.0,
                level: stage + 1,
                notes_lane,
                start: at,
                end: f64::INFINITY,
                key: event.pitch,
                velocity: event.velocity,
                pan: event.pan,
                sent: false,
                into: stage + 1,
                fed: false,
                gate: None,
                from_arp: Some(stage),
                route: None,
                echo: None,
                echo_at: f64::INFINITY,
                done: false,
            });
        }
    }

    /// Adds `note` with the next id, unless the chain is full.
    // ponytail: a full chain drops new notes; size `capacity` up if dense echo chains ever run out.
    fn add(&mut self, note: LiveNote) {
        if self.notes.len() < self.notes.capacity() {
            self.next_id = self.next_id.wrapping_add(1).max(1);
            self.notes.push(LiveNote {
                id: self.next_id,
                ..note
            });
        }
    }
}

/// Note `index` ends at `end`, known at frame `known`; so do the notes made from it, each at its start plus its
/// parent's length times its factor, but not before `known`. Notes follow their parents in `notes` (added later), and
/// ids grow along it.
// ponytail: a scan per descendant level; fine for the few hundred notes a channel has in flight.
fn end_note(notes: &mut [LiveNote], index: usize, end: f64, known: f64) {
    notes[index].end = end;
    for child in index + 1..notes.len() {
        let parent = notes[child].parent;
        if parent == 0 || notes[child].end.is_finite() {
            continue;
        }
        let Ok(at) = notes[..child].binary_search_by_key(&parent, |note| note.id) else {
            continue;
        };
        let (parent, note) = (notes[at], &mut notes[child]);
        if parent.end.is_finite() {
            note.end = (note.start + (parent.end - parent.start) * note.scale).max(known);
        }
    }
}

/// A menu's item: round(clamp(v, 0, 1)·(n − 1)) in f32, halves away from zero, as its text.
fn menu(value: f32, count: usize) -> usize {
    (value.clamp(0.0, 1.0) * (count - 1) as f32).round() as usize
}

struct Random(u64);

impl Random {
    fn new(instance_id: i32, index: usize) -> Self {
        Self(
            (instance_id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ (index as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
                ^ 0x94d0_49bb_1331_11eb,
        )
    }

    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1_u64 << 53) as f64
    }

    fn signed(&mut self) -> f64 {
        2.0 * self.unit() - 1.0
    }

    fn integer(&mut self, min: i16, max: i16) -> i16 {
        min + (self.unit() * f64::from(max - min + 1)) as i16
    }
}
