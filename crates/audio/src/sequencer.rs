use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SequencerError {
    InvalidValue,
    InvalidRhythm(u8),
    UnsupportedMode(ArpMode),
    NonContiguousBlock,
}

pub fn apply_drum_swing(
    positions_ticks: &mut [i64],
    denominator: u32,
    amount: f64,
    alternate_grid: bool,
) -> Result<(), SequencerError> {
    if denominator == 0 || !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
        return Err(SequencerError::InvalidValue);
    }
    let d = f64::from(denominator);
    for tick in positions_ticks {
        let beat = *tick as f64 / 128.0;
        let subdivision = (beat * d).round();
        let base = if alternate_grid {
            beat
        } else {
            subdivision / d
        };
        let shifted = base
            + if (subdivision as i64).rem_euclid(2) == 1 {
                0.125 * amount
            } else {
                0.0
            };
        *tick = (shifted * 128.0).round() as i64;
    }
    Ok(())
}

const RHYTHMS: [&str; 21] = [
    "10", "101", "1001", "1011", "10101", "100101", "100110", "101011", "1001010", "1001011",
    "1001101", "1010101", "1011011", "10001011", "10001101", "10010010", "10010110", "10010111",
    "10011010", "10101011", "10101101",
];

pub fn rhythm_hit(rhythm: u8, rotation: f64, step: u64) -> Result<bool, SequencerError> {
    if !rotation.is_finite() || !(0.0..=1.0).contains(&rotation) {
        return Err(SequencerError::InvalidValue);
    }
    if rhythm == 0 {
        return Ok(true);
    }
    let mask = RHYTHMS
        .get(usize::from(rhythm - 1))
        .ok_or(SequencerError::InvalidRhythm(rhythm))?;
    let length = mask.len() as i64;
    let shift = ((2.0 * rotation - 1.0) * (length - 1) as f64).round() as i64;
    let index = ((step % length as u64) as i64 - shift).rem_euclid(length) as usize;
    Ok(mask.as_bytes()[index] == b'1')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwingRate {
    Eighth,
    Sixteenth,
}

pub fn arp_swing_delay(beat: f64, interval: f64, amount: f64, rate: SwingRate) -> f64 {
    let r = match rate {
        SwingRate::Eighth => 0.5,
        SwingRate::Sixteenth => 0.25,
    };
    let phase = beat.rem_euclid(2.0 * r);
    if phase < r {
        0.0
    } else {
        ((2.0 * r - phase) / r) * (interval / 2.0).min(amount * r / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    On,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputNoteEvent {
    pub beat: f64,
    pub kind: NoteKind,
    pub pitch: i16,
    /// The note's velocity byte: an instrument gets velocity / 255.
    pub velocity: u8,
    pub pan: f32,
}

impl InputNoteEvent {
    pub fn on(beat: f64, pitch: i16, velocity: u8) -> Self {
        Self {
            beat,
            kind: NoteKind::On,
            pitch,
            velocity,
            pan: 0.5,
        }
    }
    pub fn off(beat: f64, pitch: i16) -> Self {
        Self {
            beat,
            kind: NoteKind::Off,
            pitch,
            velocity: 0,
            pan: 0.5,
        }
    }

    pub fn with_pan(mut self, pan: f32) -> Self {
        self.pan = pan;
        self
    }
}

/// Sorts `events` by beat, a note-off before a note-on at the same beat, equal ones in their given order: the stable
/// sort by that key. The note-ons and note-offs are sorted apart and merged, as each is mostly in order already (a
/// clip's note-ons are, and a note's off follows its on), which is far quicker on a song of many notes.
pub fn sort_note_events(events: &mut Vec<InputNoteEvent>) {
    let by_beat = |a: &InputNoteEvent, b: &InputNoteEvent| a.beat.total_cmp(&b.beat);
    let (mut offs, mut ons): (Vec<_>, Vec<_>) =
        events.drain(..).partition(|e| e.kind == NoteKind::Off);
    offs.sort_by(by_beat);
    ons.sort_by(by_beat);
    let mut ons = ons.into_iter().peekable();
    for off in offs {
        while let Some(on) = ons.next_if(|on| by_beat(on, &off).is_lt()) {
            events.push(on);
        }
        events.push(off);
    }
    events.extend(ons);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArpNoteEvent {
    pub beat: f64,
    pub engine: usize,
    pub kind: NoteKind,
    pub pitch: i16,
    pub velocity: u8,
    pub pan: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpMode {
    AsPlayed,
    Chord,
    Up,
    Down,
    UpDown,
    DownUp,
    Outward,
    Inward,
    Anchor,
    BassUp,
    BassUpDown,
    TrebleDown,
    TrebleDownUp,
    JumpUp,
    JumpDown,
    JumpUpDown,
    SuperSaw,
    CascadeUp,
    CascadeDown,
    CascadeUpDown,
    Random,
    RandomNoRepeat,
    RandomOnce,
    RandomWalk,
}

impl ArpMode {
    pub fn from_menu_index(index: u8) -> Result<Self, SequencerError> {
        const MODES: [ArpMode; 24] = [
            ArpMode::AsPlayed,
            ArpMode::Chord,
            ArpMode::Up,
            ArpMode::Down,
            ArpMode::UpDown,
            ArpMode::DownUp,
            ArpMode::Outward,
            ArpMode::Inward,
            ArpMode::Anchor,
            ArpMode::BassUp,
            ArpMode::BassUpDown,
            ArpMode::TrebleDown,
            ArpMode::TrebleDownUp,
            ArpMode::JumpUp,
            ArpMode::JumpDown,
            ArpMode::JumpUpDown,
            ArpMode::SuperSaw,
            ArpMode::CascadeUp,
            ArpMode::CascadeDown,
            ArpMode::CascadeUpDown,
            ArpMode::Random,
            ArpMode::RandomNoRepeat,
            ArpMode::RandomOnce,
            ArpMode::RandomWalk,
        ];
        MODES
            .get(usize::from(index))
            .copied()
            .ok_or(SequencerError::InvalidValue)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetriggerInterval {
    Eighth,
    Quarter,
    ThreeEighths,
    Half,
    ThreeQuarters,
    Bar,
    TwoBars,
    ThreeBars,
    FourBars,
    EightBars,
}

impl RetriggerInterval {
    pub fn beats(self) -> f64 {
        self.beats_for_bar(4.0)
    }

    pub fn beats_for_bar(self, bar_length_beats: f64) -> f64 {
        match self {
            Self::Eighth => 0.5,
            Self::Quarter => 1.0,
            Self::ThreeEighths => 1.5,
            Self::Half => 2.0,
            Self::ThreeQuarters => 3.0,
            Self::Bar => bar_length_beats,
            Self::TwoBars => 2.0 * bar_length_beats,
            Self::ThreeBars => 3.0 * bar_length_beats,
            Self::FourBars => 4.0 * bar_length_beats,
            Self::EightBars => 8.0 * bar_length_beats,
        }
    }
}

pub fn arp_pitch_order(
    mode: ArpMode,
    pitches: &[i16],
    range_octaves: u8,
    start_note: i8,
) -> Result<Vec<i16>, SequencerError> {
    let held = pitches
        .iter()
        .map(|pitch| (*pitch, 0, 0.5))
        .collect::<Vec<_>>();
    let mut order = Order::default();
    ordered_notes(mode, &held, range_octaves, start_note, &mut order)?;
    Ok(order.notes.into_iter().map(|note| note.0).collect())
}

/// The notes a step chooses from, into `order.notes` (its other buffers are scratch).
fn ordered_notes(
    mode: ArpMode,
    held: &[(i16, u8, f32)],
    range_octaves: u8,
    start_note: i8,
    order: &mut Order,
) -> Result<(), SequencerError> {
    if !(1..=4).contains(&range_octaves) || !(-8..=8).contains(&start_note) {
        return Err(SequencerError::InvalidValue);
    }
    if matches!(
        mode,
        ArpMode::Random | ArpMode::RandomNoRepeat | ArpMode::RandomOnce | ArpMode::RandomWalk
    ) {
        return Err(SequencerError::UnsupportedMode(mode));
    }
    let Order {
        expanded,
        indices,
        notes,
    } = order;
    expanded.clear();
    indices.clear();
    notes.clear();
    let mut seen = [false; 128];
    for &(pitch, velocity, pan) in held {
        if !(0..=127).contains(&pitch) {
            return Err(SequencerError::InvalidValue);
        }
        for octave in 0..range_octaves {
            let shifted = pitch + i16::from(octave) * 12;
            if shifted <= 127 && !std::mem::replace(&mut seen[shifted as usize], true) {
                expanded.push((shifted, velocity, pan));
            }
        }
    }
    if expanded.is_empty() {
        return Ok(());
    }
    if mode == ArpMode::AsPlayed {
        notes.extend_from_slice(expanded);
        return Ok(());
    }
    // The pitches are distinct, so an unstable sort (which never allocates) orders them as a stable one would.
    expanded.sort_unstable_by_key(|note| note.0);
    let n = expanded.len();
    if mode == ArpMode::SuperSaw {
        if n == 1 {
            notes.extend_from_slice(expanded);
            return Ok(());
        }
        let range = usize::from(range_octaves);
        let groups = if n == 2 {
            range / if range % 2 == 0 { 2 } else { 1 }
        } else {
            (n - 2) * range
        };
        let divisor = if n == 2 { 1 } else { 3 * n - 6 };
        let mut counter = 0;
        for group in 0..groups {
            let j = if n == 2 { 1 } else { 2 + group % (n - 2) };
            for index in [0, 1, j - 1, j] {
                let (pitch, velocity, pan) = expanded[index];
                let transpose = if index == 0 {
                    0
                } else {
                    let octave = (counter / divisor) % range;
                    counter += 1;
                    (octave * 12) as i16
                };
                if pitch + transpose <= 127 {
                    notes.push((pitch + transpose, velocity, pan));
                }
            }
        }
        return Ok(());
    }
    if mode == ArpMode::Anchor {
        let index = isize::from(start_note).rem_euclid(n as isize) as usize;
        let (pitch, velocity, pan) = expanded[index];
        notes.extend(
            (0..range_octaves)
                .map(|octave| (pitch + i16::from(octave) * 12, velocity, pan))
                .filter(|note| note.0 <= 127),
        );
        return Ok(());
    }
    match mode {
        ArpMode::Chord | ArpMode::Up => indices.extend(0..n),
        ArpMode::Down => indices.extend((0..n).rev()),
        ArpMode::UpDown => {
            indices.extend(0..n);
            indices.extend((1..n.saturating_sub(1)).rev());
        }
        ArpMode::DownUp => {
            indices.extend((0..n).rev());
            indices.extend(1..n.saturating_sub(1));
        }
        ArpMode::Outward => {
            let mut left = (n as isize - 1) / 2;
            let mut right = n / 2;
            while left >= 0 || right < n {
                if left >= 0 {
                    indices.push(left as usize);
                }
                if right < n && right as isize != left {
                    indices.push(right);
                }
                left -= 1;
                right += 1;
            }
        }
        ArpMode::Inward => {
            let mut left = 0;
            let mut right = n - 1;
            while left <= right {
                indices.push(left);
                if left != right {
                    indices.push(right);
                }
                left += 1;
                right = right.saturating_sub(1);
            }
        }
        ArpMode::BassUp => {
            if n == 1 {
                indices.push(0);
            } else {
                for index in 1..n {
                    indices.extend([0, index]);
                }
            }
        }
        ArpMode::BassUpDown => {
            if n == 1 {
                indices.push(0);
            } else {
                for index in 1..n {
                    indices.extend([0, index]);
                }
                for index in (2..n.saturating_sub(1)).rev() {
                    indices.extend([0, index]);
                }
            }
        }
        ArpMode::TrebleDown => {
            if n == 1 {
                indices.push(0);
            } else {
                for index in (0..n - 1).rev() {
                    indices.extend([n - 1, index]);
                }
            }
        }
        ArpMode::TrebleDownUp => {
            if n == 1 {
                indices.push(0);
            } else {
                for index in (0..n - 1).rev() {
                    indices.extend([n - 1, index]);
                }
                for index in 1..n.saturating_sub(2) {
                    indices.extend([n - 1, index]);
                }
            }
        }
        ArpMode::JumpUp => {
            indices.extend((0..n).step_by(2));
            indices.extend((1..n).step_by(2));
        }
        ArpMode::JumpDown => {
            indices.extend((0..n).rev().step_by(2));
            indices.extend((0..n).rev().skip(1).step_by(2));
        }
        ArpMode::JumpUpDown => {
            indices.extend((0..n).step_by(2));
            indices.extend((1..n).step_by(2));
            indices.extend((0..n).rev().step_by(2));
            indices.extend((0..n).rev().skip(1).step_by(2));
        }
        ArpMode::CascadeUp => cascade_indices(indices, n, true, true),
        ArpMode::CascadeDown => cascade_indices(indices, n, false, true),
        ArpMode::CascadeUpDown => {
            cascade_indices(indices, n, true, true);
            cascade_indices(indices, n, false, false);
        }
        mode => return Err(SequencerError::UnsupportedMode(mode)),
    }
    notes.extend(indices.iter().map(|&index| expanded[index]));
    Ok(())
}

fn cascade_indices(indices: &mut Vec<usize>, len: usize, up: bool, include_first: bool) {
    let mut index = if up { 0 } else { len as isize - 1 };
    let deltas = if up { [1, 1, -1] } else { [-1, -1, 1] };
    if include_first {
        indices.push(index as usize);
    }
    let mut phase = 0;
    loop {
        index += deltas[phase];
        if index < 0 || index >= len as isize {
            break;
        }
        indices.push(index as usize);
        phase = (phase + 1) % 3;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineSettings {
    pub enabled: bool,
    pub mode: ArpMode,
    pub rhythm: u8,
    pub rotation: f64,
    pub rate_multiplier: usize,
    pub gate: f64,
    pub velocity_adjustment: f64,
    pub octave_shift: i8,
    pub range_octaves: u8,
    pub start_note: i8,
    pub repeats: Option<u8>,
    pub retrigger_on_notes: bool,
    pub retrigger_interval: Option<RetriggerInterval>,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: ArpMode::AsPlayed,
            rhythm: 0,
            rotation: 0.5,
            rate_multiplier: 6,
            gate: 0.5,
            velocity_adjustment: 0.0,
            octave_shift: 0,
            range_octaves: 1,
            start_note: 0,
            repeats: None,
            retrigger_on_notes: false,
            retrigger_interval: None,
        }
    }
}

impl EngineSettings {
    pub fn set_retrigger_menu_index(&mut self, index: u8) -> Result<(), SequencerError> {
        let interval = match index {
            0 | 1 => None,
            2 => Some(RetriggerInterval::Eighth),
            3 => Some(RetriggerInterval::Quarter),
            4 => Some(RetriggerInterval::ThreeEighths),
            5 => Some(RetriggerInterval::Half),
            6 => Some(RetriggerInterval::ThreeQuarters),
            7 => Some(RetriggerInterval::Bar),
            8 => Some(RetriggerInterval::TwoBars),
            9 => Some(RetriggerInterval::ThreeBars),
            10 => Some(RetriggerInterval::FourBars),
            11 => Some(RetriggerInterval::EightBars),
            _ => return Err(SequencerError::InvalidValue),
        };
        self.retrigger_on_notes = index == 1;
        self.retrigger_interval = interval;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArpSettings {
    pub sync: bool,
    pub sync_rate: usize,
    pub free_rate_ms: f64,
    pub swing: f64,
    pub swing_rate: SwingRate,
    pub latch: bool,
    pub bar_length_beats: f64,
    pub engines: [EngineSettings; 2],
}

impl Default for ArpSettings {
    fn default() -> Self {
        let second = EngineSettings {
            enabled: false,
            ..EngineSettings::default()
        };
        Self {
            sync: true,
            sync_rate: 2,
            free_rate_ms: 250.0,
            swing: 0.0,
            swing_rate: SwingRate::Eighth,
            latch: false,
            bar_length_beats: 4.0,
            engines: [EngineSettings::default(), second],
        }
    }
}

impl ArpSettings {
    pub fn step_interval(&self, engine: usize, tempo_bpm: f64) -> Result<f64, SequencerError> {
        const RATES: [f64; 8] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0, 4.0 / 3.0, 2.0, 4.0];
        const MULTIPLIERS: [f64; 15] = [
            1.0 / 8.0,
            1.0 / 6.0,
            0.25,
            1.0 / 3.0,
            0.5,
            2.0 / 3.0,
            1.0,
            1.5,
            2.0,
            3.0,
            4.0,
            5.0,
            6.0,
            7.0,
            8.0,
        ];
        let e = self
            .engines
            .get(engine)
            .ok_or(SequencerError::InvalidValue)?;
        let multiplier = MULTIPLIERS
            .get(e.rate_multiplier)
            .ok_or(SequencerError::InvalidValue)?;
        if !tempo_bpm.is_finite() || tempo_bpm <= 0.0 {
            return Err(SequencerError::InvalidValue);
        }
        let base = if self.sync {
            *RATES
                .get(self.sync_rate)
                .ok_or(SequencerError::InvalidValue)?
        } else {
            if !self.free_rate_ms.is_finite() || !(50.0..=1000.0).contains(&self.free_rate_ms) {
                return Err(SequencerError::InvalidValue);
            }
            self.free_rate_ms * tempo_bpm / 60_000.0
        };
        Ok(base * multiplier)
    }

    fn validate(&self, tempo_bpm: f64) -> Result<(), SequencerError> {
        if !self.swing.is_finite()
            || !(0.0..=1.0).contains(&self.swing)
            || !self.bar_length_beats.is_finite()
            || self.bar_length_beats <= 0.0
        {
            return Err(SequencerError::InvalidValue);
        }
        for (i, e) in self.engines.iter().enumerate() {
            self.step_interval(i, tempo_bpm)?;
            if !e.rotation.is_finite()
                || !(0.0..=1.0).contains(&e.rotation)
                || !e.gate.is_finite()
                || !(0.1..=1.0).contains(&e.gate)
                || !e.velocity_adjustment.is_finite()
                || !(-1.0..=1.0).contains(&e.velocity_adjustment)
                || !(-3..=3).contains(&e.octave_shift)
                || !(1..=4).contains(&e.range_octaves)
                || !(-8..=8).contains(&e.start_note)
                || (e.retrigger_on_notes && e.retrigger_interval.is_some())
                || e.repeats == Some(0)
                || e.repeats.is_some_and(|n| n > 16)
            {
                return Err(SequencerError::InvalidValue);
            }
            rhythm_hit(e.rhythm, e.rotation, 0)?;
        }
        Ok(())
    }
}

/// Where an arp engine's step clock stands from `beat` on, while traced ([`Arpeggiator::trace`]): its origin (`None`:
/// no notes held, the clock stopped), its step interval in beats, and whether the engine is on. Step k sounds at
/// origin + k·interval, so the playhead at beat b is (b − origin)/interval steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockMark {
    pub beat: f64,
    pub origin: Option<f64>,
    pub interval: f64,
    pub enabled: bool,
}

#[derive(Default)]
struct EngineState {
    /// The clock marks so far, while traced.
    clock: Option<Vec<ClockMark>>,
    /// The interval and on state of the block being processed, for the marks.
    timing: (f64, bool),
    held: Vec<(i16, u8, f32)>,
    origin: Option<f64>,
    next_step: u64,
    pitch_index: usize,
    completed_cycles: u64,
    pending: Vec<ArpNoteEvent>,
    random_state: u64,
    random_mode: Option<ArpMode>,
    random_source: Vec<i16>,
    random_bag: Vec<usize>,
    random_cursor: usize,
    random_last_pitch: Option<i16>,
    random_direction: i8,
    /// Scratch for [`ordered_notes`], reused so a step allocates nothing.
    order: Order,
}

/// [`ordered_notes`]' buffers: the held notes over the range, a pattern's indices into them, and the result.
#[derive(Default)]
struct Order {
    expanded: Vec<(i16, u8, f32)>,
    indices: Vec<usize>,
    notes: Vec<(i16, u8, f32)>,
}

/// Room reserved up front, so the live arpeggiator never allocates: 128 held keys, and a pattern over every key in
/// four octaves (SuperSaw's groups run longest; a larger chord only reallocates).
const HELD: usize = 128;
const STEPS: usize = 4096;

impl EngineState {
    fn new(random_state: u64) -> Self {
        Self {
            held: Vec::with_capacity(HELD),
            pending: Vec::with_capacity(STEPS),
            random_state,
            random_source: Vec::with_capacity(STEPS),
            random_bag: Vec::with_capacity(STEPS),
            order: Order {
                expanded: Vec::with_capacity(STEPS),
                indices: Vec::with_capacity(STEPS),
                notes: Vec::with_capacity(STEPS),
            },
            ..Self::default()
        }
    }

    /// Back to [`Self::new`]'s state in place, keeping the buffers.
    fn clear(&mut self, random_state: u64) {
        self.held.clear();
        self.pending.clear();
        self.random_source.clear();
        self.random_bag.clear();
        self.origin = None;
        self.next_step = 0;
        self.pitch_index = 0;
        self.completed_cycles = 0;
        self.random_state = random_state;
        self.random_mode = None;
        self.random_cursor = 0;
        self.random_last_pitch = None;
        self.random_direction = 0;
    }
}

#[derive(Clone, Copy)]
struct ArpTiming {
    interval: f64,
    bar_length_beats: f64,
    swing: (f64, SwingRate),
    block_start: f64,
}

pub struct Arpeggiator {
    pub settings: ArpSettings,
    engines: [EngineState; 2],
    end_beat: Option<f64>,
    random_seed: u64,
}

impl Arpeggiator {
    pub fn new(settings: ArpSettings) -> Self {
        Self::with_random_seed(settings, 0)
    }

    pub fn with_random_seed(settings: ArpSettings, seed: u64) -> Self {
        Self {
            settings,
            engines: [0, 1].map(|index| EngineState::new(random_state(seed, index))),
            end_beat: None,
            random_seed: seed,
        }
    }

    /// Back to the state [`Self::with_random_seed`] made, settings kept; allocates nothing.
    pub fn reset(&mut self) {
        for (index, engine) in self.engines.iter_mut().enumerate() {
            engine.clear(random_state(self.random_seed, index as u64));
        }
        self.end_beat = None;
    }

    /// The earliest beat at or after the last block's end where [`Self::process_block`] can return an event: a pending
    /// note, a step or a retrigger boundary (which starts a step). `None` while nothing is held or pending.
    pub fn next_event_beat(&self, tempo_bpm: f64) -> Option<f64> {
        let mut next = f64::INFINITY;
        for (engine, state) in self.engines.iter().enumerate() {
            for event in &state.pending {
                next = next.min(event.beat);
            }
            let (Some(origin), Ok(interval)) =
                (state.origin, self.settings.step_interval(engine, tempo_bpm))
            else {
                continue;
            };
            next = next.min(origin + state.next_step as f64 * interval);
            if let Some(retrig) = self.settings.engines[engine].retrigger_interval {
                let period = retrig
                    .beats_for_bar(self.settings.bar_length_beats)
                    .max(interval);
                let start = self.end_beat.unwrap_or(origin);
                let mut boundary = (origin.max(start) / period).ceil() * period;
                if boundary <= origin + 1e-9 {
                    boundary += period;
                }
                next = next.min(boundary);
            }
        }
        next.is_finite().then_some(next)
    }

    /// Keeps each engine's [`ClockMark`]s from now on (for its panel's playhead); [`Self::clock`] reads them.
    pub fn trace(&mut self) {
        for engine in &mut self.engines {
            engine.clock = Some(Vec::new());
        }
    }

    /// Engine `engine`'s clock marks in the order they were made (by beat), while traced; empty otherwise.
    pub fn clock(&self, engine: usize) -> &[ClockMark] {
        self.engines[engine].clock.as_deref().unwrap_or_default()
    }

    pub fn process_block(
        &mut self,
        start: f64,
        end: f64,
        tempo_bpm: f64,
        input: &[InputNoteEvent],
    ) -> Result<Vec<ArpNoteEvent>, SequencerError> {
        let mut result = Vec::new();
        self.schedule(start, end, tempo_bpm, input, &mut result)?;
        result.sort_by(arp_order);
        Ok(result)
    }

    /// [`Self::process_block`] appending to `output`, for the audio thread: it allocates nothing while `output` has
    /// room. The appended events are in the same order (an insertion sort, stable as `sort_by`).
    pub fn process_block_into(
        &mut self,
        start: f64,
        end: f64,
        tempo_bpm: f64,
        input: &[InputNoteEvent],
        output: &mut Vec<ArpNoteEvent>,
    ) -> Result<(), SequencerError> {
        let first = output.len();
        self.schedule(start, end, tempo_bpm, input, output)?;
        let appended = &mut output[first..];
        for i in 1..appended.len() {
            let mut j = i;
            while j > 0 && arp_order(&appended[j - 1], &appended[j]).is_gt() {
                appended.swap(j - 1, j);
                j -= 1;
            }
        }
        Ok(())
    }

    /// The events of [`Self::process_block`], unsorted, appended to `result`.
    fn schedule(
        &mut self,
        start: f64,
        end: f64,
        tempo_bpm: f64,
        input: &[InputNoteEvent],
        result: &mut Vec<ArpNoteEvent>,
    ) -> Result<(), SequencerError> {
        if !start.is_finite()
            || !end.is_finite()
            || end <= start
            || self
                .end_beat
                .is_some_and(|previous| (previous - start).abs() > 1e-9)
        {
            return Err(SequencerError::NonContiguousBlock);
        }
        self.settings.validate(tempo_bpm)?;
        let mut previous = start;
        for event in input {
            if !event.beat.is_finite()
                || event.beat < previous
                || event.beat >= end
                || !event.pan.is_finite()
                || !(0.0..=1.0).contains(&event.pan)
            {
                return Err(SequencerError::InvalidValue);
            }
            previous = event.beat;
        }
        for engine in 0..2 {
            let config = self.settings.engines[engine];
            let interval = self.settings.step_interval(engine, tempo_bpm)?;
            let timing = ArpTiming {
                interval,
                bar_length_beats: self.settings.bar_length_beats,
                swing: (self.settings.swing, self.settings.swing_rate),
                block_start: start,
            };
            let state = &mut self.engines[engine];
            state.timing = (interval, config.enabled);
            state.mark(start);
            if let Some(origin) = state.origin {
                while origin + (state.next_step as f64) * interval < start {
                    state.next_step += 1;
                }
                // A slower rate since the last block (automation): the steps from `start` on are still to come.
                while state.next_step > 0
                    && origin + (state.next_step - 1) as f64 * interval >= start
                {
                    state.next_step -= 1;
                }
            }
            let mut cursor = 0;
            while cursor < input.len() {
                let beat = input[cursor].beat;
                state.schedule_until(beat, false, engine, config, timing)?;
                while cursor < input.len() && input[cursor].beat == beat {
                    state.input(input[cursor], self.settings.latch, config);
                    cursor += 1;
                }
                state.schedule_until(beat, true, engine, config, timing)?;
            }
            state.schedule_until(end, false, engine, config, timing)?;
            state.pending.retain(|event| {
                let due = event.beat < end;
                if due {
                    result.push(*event);
                }
                !due
            });
        }
        self.end_beat = Some(end);
        Ok(())
    }
}

/// The arpeggiator's output order: by beat, a note-off before a note-on, then by engine.
fn arp_order(a: &ArpNoteEvent, b: &ArpNoteEvent) -> std::cmp::Ordering {
    a.beat
        .total_cmp(&b.beat)
        .then_with(|| (a.kind != NoteKind::Off).cmp(&(b.kind != NoteKind::Off)))
        .then(a.engine.cmp(&b.engine))
}

/// Engine `index`'s random state for `seed`, never 0.
fn random_state(seed: u64, index: u64) -> u64 {
    let state = seed.wrapping_add(index) % 2_147_483_647;
    if state == 0 { 1 } else { state }
}

impl EngineState {
    /// Records where the clock stands from `beat` on, while traced.
    fn mark(&mut self, beat: f64) {
        let (interval, enabled) = self.timing;
        let origin = self.origin;
        if let Some(clock) = &mut self.clock {
            clock.push(ClockMark {
                beat,
                origin,
                interval,
                enabled,
            });
        }
    }

    fn reset_sequence(&mut self, beat: f64) {
        self.origin = Some(beat);
        self.mark(beat);
        self.next_step = 0;
        self.pitch_index = 0;
        self.completed_cycles = 0;
        self.random_cursor = 0;
        self.random_last_pitch = None;
        self.random_direction = 0;
    }

    fn input(&mut self, event: InputNoteEvent, latch: bool, config: EngineSettings) {
        match event.kind {
            NoteKind::On => {
                self.held.retain(|(pitch, _, _)| *pitch != event.pitch);
                self.held.push((event.pitch, event.velocity, event.pan));
                if self.origin.is_none() || config.retrigger_on_notes {
                    self.reset_sequence(event.beat);
                }
            }
            NoteKind::Off if !latch => {
                self.held.retain(|(pitch, _, _)| *pitch != event.pitch);
                if self.held.is_empty() {
                    self.origin = None;
                    self.mark(event.beat);
                    self.pitch_index = 0;
                    self.completed_cycles = 0;
                    self.random_cursor = 0;
                    self.random_last_pitch = None;
                    self.random_direction = 0;
                    self.random_source.clear();
                    self.random_bag.clear();
                    self.random_mode = None;
                } else if config.retrigger_on_notes {
                    self.reset_sequence(event.beat);
                }
            }
            NoteKind::Off => {}
        }
    }

    fn schedule_until(
        &mut self,
        limit: f64,
        inclusive: bool,
        engine: usize,
        config: EngineSettings,
        timing: ArpTiming,
    ) -> Result<(), SequencerError> {
        let Some(_) = self.origin else {
            return Ok(());
        };
        loop {
            let origin = self.origin.unwrap();
            let beat = origin + self.next_step as f64 * timing.interval;
            let boundary = config.retrigger_interval.map(|retrig| {
                let period = retrig
                    .beats_for_bar(timing.bar_length_beats)
                    .max(timing.interval);
                let mut next = ((origin.max(timing.block_start) / period).ceil()) * period;
                if next <= origin + 1e-9 {
                    next += period;
                }
                next
            });
            let next = boundary.map_or(beat, |boundary| boundary.min(beat));
            if next > limit || (!inclusive && next >= limit) {
                break;
            }
            if boundary.is_some_and(|boundary| boundary <= beat + 1e-9) {
                self.reset_sequence(boundary.unwrap());
                continue;
            }
            self.next_step += 1;
            if !config.enabled
                || self.held.is_empty()
                || !rhythm_hit(config.rhythm, config.rotation, self.next_step - 1)?
                || config
                    .repeats
                    .is_some_and(|n| self.completed_cycles >= u64::from(n))
            {
                continue;
            }
            let random_mode = matches!(
                config.mode,
                ArpMode::Random
                    | ArpMode::RandomNoRepeat
                    | ArpMode::RandomOnce
                    | ArpMode::RandomWalk
            );
            ordered_notes(
                if random_mode {
                    ArpMode::Up
                } else {
                    config.mode
                },
                &self.held,
                config.range_octaves,
                config.start_note,
                &mut self.order,
            )?;
            // Out of `self` while `random_select` borrows it; put back below, so its room is kept.
            let notes = std::mem::take(&mut self.order.notes);
            if notes.is_empty() {
                self.order.notes = notes;
                continue;
            }
            let cycle_len = if config.mode == ArpMode::Chord {
                1
            } else {
                notes.len()
            };
            let one;
            let selected: &[(i16, u8, f32)] = if random_mode {
                one = [self.random_select(config.mode, &notes)];
                &one
            } else if config.mode == ArpMode::Chord {
                &notes
            } else {
                let start_note = if config.mode == ArpMode::Anchor {
                    0
                } else {
                    isize::from(config.start_note)
                };
                let index = (self.pitch_index as isize + start_note).rem_euclid(cycle_len as isize)
                    as usize;
                one = [notes[index]];
                &one
            };
            let on_beat =
                beat + arp_swing_delay(beat, timing.interval, timing.swing.0, timing.swing.1);
            for &(pitch, velocity, pan) in selected {
                let pitch = pitch + i16::from(config.octave_shift) * 12;
                let velocity = (f64::from(velocity) * (1.0 + config.velocity_adjustment))
                    .round()
                    .clamp(0.0, 255.0) as u8;
                self.pending.push(ArpNoteEvent {
                    beat: on_beat,
                    engine,
                    kind: NoteKind::On,
                    pitch,
                    velocity,
                    pan,
                });
                self.pending.push(ArpNoteEvent {
                    beat: on_beat + timing.interval * config.gate,
                    engine,
                    kind: NoteKind::Off,
                    pitch,
                    velocity: 0,
                    pan,
                });
            }
            self.order.notes = notes;
            self.pitch_index = (self.pitch_index + 1) % cycle_len;
            if self.pitch_index == 0 {
                self.completed_cycles += 1;
            }
        }
        Ok(())
    }

    fn random_u64(&mut self) -> u64 {
        self.random_state = self.random_state * 48_271 % 2_147_483_647;
        self.random_state
    }

    fn random_below(&mut self, count: usize) -> usize {
        let span = 2_147_483_646_u64;
        let bucket = span / count as u64;
        loop {
            let value = self.random_u64() - 1;
            if value < bucket * count as u64 {
                return (value / bucket) as usize;
            }
        }
    }

    fn shuffle_bag(&mut self) {
        for index in (1..self.random_bag.len()).rev() {
            let swap = self.random_below(index + 1);
            self.random_bag.swap(index, swap);
        }
    }

    fn random_select(&mut self, mode: ArpMode, notes: &[(i16, u8, f32)]) -> (i16, u8, f32) {
        let same_source = self.random_source.len() == notes.len()
            && (self.random_source.iter().zip(notes)).all(|(pitch, note)| *pitch == note.0);
        if self.random_mode != Some(mode) || !same_source {
            if self.random_mode != Some(mode) {
                self.random_last_pitch = None;
                self.random_direction = 0;
            } else if mode == ArpMode::RandomWalk {
                self.random_direction = 0;
            }
            self.random_bag.clear();
            self.random_bag.extend(0..notes.len());
            self.random_cursor = 0;
            if mode == ArpMode::RandomOnce {
                self.shuffle_bag();
            }
            if self
                .random_last_pitch
                .is_some_and(|pitch| !notes.iter().any(|note| note.0 == pitch))
            {
                self.random_last_pitch = None;
            }
            self.random_source.clear();
            self.random_source.extend(notes.iter().map(|note| note.0));
            self.random_mode = Some(mode);
        }
        let len = notes.len();
        let index = match mode {
            ArpMode::Random => self.random_below(len),
            ArpMode::RandomNoRepeat => {
                if self.random_cursor == 0 || self.random_cursor >= len {
                    self.shuffle_bag();
                    if len > 1
                        && self.random_last_pitch == Some(notes[self.random_bag[0]].0)
                        && let Some(other) = (1..len).find(|&i| {
                            notes[self.random_bag[i]].0 != self.random_last_pitch.unwrap()
                        })
                    {
                        self.random_bag.swap(0, other);
                    }
                    self.random_cursor = 0;
                }
                let selected = self.random_bag[self.random_cursor];
                self.random_cursor += 1;
                selected
            }
            ArpMode::RandomOnce => {
                let selected = self.random_bag[self.random_cursor];
                self.random_cursor = (self.random_cursor + 1) % len;
                selected
            }
            ArpMode::RandomWalk => {
                if len < 2 {
                    0
                } else if let Some(previous) = self
                    .random_last_pitch
                    .and_then(|pitch| notes.iter().position(|note| note.0 == pitch))
                {
                    if self.random_direction == 0 {
                        self.random_direction = if self.random_below(2) == 0 { -1 } else { 1 };
                    }
                    if previous == 0 {
                        self.random_direction = 1;
                    } else if previous == len - 1 {
                        self.random_direction = -1;
                    }
                    let direction = isize::from(self.random_direction);
                    let (mut choices, mut count) = ([(0, 0); 3], 0);
                    for (delta, weight) in [(direction, 50), (-direction, 30), (2 * direction, 20)]
                    {
                        let candidate = previous as isize + delta;
                        if (0..len as isize).contains(&candidate) {
                            choices[count] = (candidate as usize, weight);
                            count += 1;
                        }
                    }
                    let choices = &choices[..count];
                    let total: usize = choices.iter().map(|choice| choice.1).sum();
                    let mut draw = self.random_below(total) + 1;
                    let mut selected = previous;
                    for &(candidate, weight) in choices {
                        if draw <= weight {
                            selected = candidate;
                            break;
                        }
                        draw -= weight;
                    }
                    self.random_direction = if selected < previous { -1 } else { 1 };
                    selected
                } else {
                    let selected = self.random_below(len);
                    self.random_direction = if self.random_below(2) == 0 { -1 } else { 1 };
                    selected
                }
            }
            _ => unreachable!(),
        };
        self.random_last_pitch = Some(notes[index].0);
        notes[index]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedNoteEvent {
    pub at: Duration,
    pub kind: NoteKind,
    pub pitch: i16,
    pub velocity: u8,
}

impl TimedNoteEvent {
    pub fn on(at: Duration, pitch: i16, velocity: u8) -> Self {
        Self {
            at,
            kind: NoteKind::On,
            pitch,
            velocity,
        }
    }
    pub fn off(at: Duration, pitch: i16) -> Self {
        Self {
            at,
            kind: NoteKind::Off,
            pitch,
            velocity: 0,
        }
    }
}

pub fn chord_onsets(
    pitches: &[i16],
    timing: f64,
    octave_shift: i8,
    velocity: u8,
    at: Duration,
) -> Result<Vec<TimedNoteEvent>, SequencerError> {
    if !timing.is_finite() || !(0.0..=1.0).contains(&timing) || velocity > 127 {
        return Err(SequencerError::InvalidValue);
    }
    let mut pitches = pitches.to_vec();
    pitches.sort_unstable();
    pitches.dedup();
    if timing < 0.5 {
        pitches.reverse();
    }
    let spacing = Duration::from_secs_f64(((2.0 * timing - 1.0) * 0.040).abs());
    Ok(pitches
        .into_iter()
        .enumerate()
        .map(|(i, pitch)| {
            TimedNoteEvent::on(
                at + spacing * i as u32,
                pitch + i16::from(octave_shift) * 12,
                velocity,
            )
        })
        .collect())
}

#[derive(Default)]
pub struct ChordKeyboard {
    strips: BTreeMap<u64, BTreeSet<i16>>,
    owners: BTreeMap<i16, usize>,
    sounding: BTreeSet<i16>,
    queued: Vec<(u64, TimedNoteEvent)>,
    deferred: BTreeSet<u64>,
    hold: bool,
}

impl ChordKeyboard {
    pub fn touch(
        &mut self,
        strip: u64,
        pitches: &[i16],
        timing: f64,
        octave_shift: i8,
        velocity: u8,
        at: Duration,
    ) -> Result<Vec<TimedNoteEvent>, SequencerError> {
        if self.strips.contains_key(&strip) {
            return Err(SequencerError::InvalidValue);
        }
        self.change(strip, pitches, timing, octave_shift, velocity, at)
    }

    pub fn slide(
        &mut self,
        strip: u64,
        pitches: &[i16],
        timing: f64,
        octave_shift: i8,
        velocity: u8,
        at: Duration,
    ) -> Result<Vec<TimedNoteEvent>, SequencerError> {
        if !self.strips.contains_key(&strip) {
            return Err(SequencerError::InvalidValue);
        }
        self.change(strip, pitches, timing, octave_shift, velocity, at)
    }

    fn change(
        &mut self,
        strip: u64,
        pitches: &[i16],
        timing: f64,
        octave_shift: i8,
        velocity: u8,
        at: Duration,
    ) -> Result<Vec<TimedNoteEvent>, SequencerError> {
        let starts = chord_onsets(pitches, timing, octave_shift, velocity, at)?;
        let new: BTreeSet<_> = starts.iter().map(|event| event.pitch).collect();
        let old = self.strips.insert(strip, new.clone()).unwrap_or_default();
        self.deferred.remove(&strip);
        self.queued.retain(|(owner, event)| {
            *owner != strip || event.at < at || new.contains(&event.pitch)
        });
        let mut output = self.advance(at);
        for pitch in old.difference(&new) {
            self.remove_owner(*pitch, at, &mut output);
        }
        for event in starts {
            if old.contains(&event.pitch) {
                continue;
            }
            *self.owners.entry(event.pitch).or_default() += 1;
            if event.at <= at {
                self.sound(event, &mut output);
            } else {
                self.queued.push((strip, event));
            }
        }
        Ok(output)
    }

    pub fn advance(&mut self, now: Duration) -> Vec<TimedNoteEvent> {
        self.queued.sort_by_key(|(_, event)| event.at);
        let mut output = Vec::new();
        let mut later = Vec::new();
        for (strip, event) in std::mem::take(&mut self.queued) {
            if event.at <= now {
                self.sound(event, &mut output);
            } else {
                later.push((strip, event));
            }
        }
        self.queued = later;
        output
    }

    fn sound(&mut self, event: TimedNoteEvent, output: &mut Vec<TimedNoteEvent>) {
        if self.sounding.contains(&event.pitch) {
            output.push(TimedNoteEvent::off(event.at, event.pitch));
        }
        self.sounding.insert(event.pitch);
        output.push(event);
    }

    fn remove_owner(&mut self, pitch: i16, at: Duration, output: &mut Vec<TimedNoteEvent>) {
        if let Some(count) = self.owners.get_mut(&pitch) {
            *count -= 1;
            if *count == 0 {
                self.owners.remove(&pitch);
                if self.sounding.remove(&pitch) {
                    output.push(TimedNoteEvent::off(at, pitch));
                }
            }
        }
    }

    pub fn release(&mut self, strip: u64, at: Duration) -> Vec<TimedNoteEvent> {
        if self.hold {
            self.deferred.insert(strip);
            return Vec::new();
        }
        self.queued
            .retain(|(owner, event)| *owner != strip || event.at < at);
        let mut output = self.advance(at);
        if let Some(pitches) = self.strips.remove(&strip) {
            for pitch in pitches {
                self.remove_owner(pitch, at, &mut output);
            }
        }
        output
    }

    pub fn set_hold(&mut self, hold: bool, at: Duration) -> Vec<TimedNoteEvent> {
        self.hold = hold;
        if hold {
            return Vec::new();
        }
        let mut output = Vec::new();
        for strip in std::mem::take(&mut self.deferred) {
            output.extend(self.release(strip, at));
        }
        output
    }
}

#[derive(Default)]
pub struct StrumKeyboard {
    fingers: BTreeMap<u64, (u64, i16)>,
}

impl StrumKeyboard {
    pub fn enter_cell(
        &mut self,
        finger: u64,
        cell: u64,
        pitch: i16,
        octave_shift: i8,
        velocity: u8,
        at: Duration,
    ) -> Vec<TimedNoteEvent> {
        let pitch = pitch + i16::from(octave_shift) * 12;
        if self.fingers.get(&finger) == Some(&(cell, pitch)) {
            return Vec::new();
        }
        let mut output = Vec::new();
        if let Some((_, old_pitch)) = self.fingers.insert(finger, (cell, pitch)) {
            output.push(TimedNoteEvent::off(at, old_pitch));
        }
        output.push(TimedNoteEvent::on(at, pitch, velocity));
        output
    }

    pub fn release(&mut self, finger: u64, at: Duration) -> Vec<TimedNoteEvent> {
        self.fingers
            .remove(&finger)
            .map(|(_, pitch)| vec![TimedNoteEvent::off(at, pitch)])
            .unwrap_or_default()
    }
}
