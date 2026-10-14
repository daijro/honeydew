//! What an effect's panel draws from the audio besides its meter: the effect's analysis, which the audio thread
//! keeps up to date as the effect runs. It is a slab of f32 laid out per module type, sized when the effect is built,
//! so nothing here allocates on the audio thread. The device publishes it lock-free (`device::MeterBank`), and the
//! panels read it back through the views at the end of this file, which own each layout.
//!
//! The analyses observe the effect; none changes what it sounds like.

pub mod analyzer;

use crate::Processor;
use crate::biquad::flush;
use crate::note_effects::NoteEffectTrace;
use crate::pitcher::PitcherState;
use analyzer::Analyzer;

/// A history display's columns: 6.4 s at one entry per 50 ms.
pub const HISTORY: usize = 128;
/// The Wow & Flutter trace's averaged points (600 points of 200 entries each cover the 120 000-entry
/// ring, so a panel averages points, not samples, into its 2 px columns).
pub const TRACE_POINTS: usize = 600;
/// The trace entries (samples) each point averages.
pub const TRACE_ENTRIES: usize = 200;
/// The vectorscope's direction bins, 15° apart from 15° to 165°.
pub const PETAL_BINS: usize = 11;
/// Samples per vectorscope and correlation block.
pub const BLOCK: usize = 2048;

/// A level as a history entry stores it: f = min(20·log10(v)/66 + 60/66, 1), v ≤ 0.001 taken as −60 dB.
pub fn history_level(v: f32) -> f32 {
    if v > 0.001 {
        (20.0 * v.log10() / 66.0 + 60.0 / 66.0).min(1.0)
    } else {
        0.0
    }
}

/// An effect's analysis: its type's state and the slab it publishes.
pub(crate) struct Probe {
    kind: Kind,
    slab: Vec<f32>,
}

enum Kind {
    None,
    /// Gate (27): the detector's RMS levels, dB.
    Gate,
    /// Limiter (28): the 6.4 s history.
    Limiter(History),
    /// Multiband Compressor (36): one history per band.
    Multiband(Box<[History; 3]>),
    /// Spacer (34): the input's and the output's L/R correlation.
    Spacer(Box<[Correlation; 2]>),
    /// Spreader (44): the vectorscope of the output.
    Spreader(Box<Petal>),
    /// Wow & Flutter (43): the modulated delay's trace.
    WowFlutter(Trace),
    /// Tremolo (46): the left oscillator's phase.
    Tremolo,
    /// Tape Stop (42): the spools' angle, degrees.
    TapeStop(f64),
    /// Waveshaper (41): the effective X and Y.
    Waveshaper,
    /// Tuner (24): its pitch detector.
    Tuner(Box<Detector>),
    /// Pitcher (25) and Pitcher 2 (45): the trackers' pitch and target.
    Pitcher,
    Pitcher2,
    /// Analyzer (26): every view's data ([`analyzer`]).
    Analyzer(Box<Analyzer>),
}

impl Probe {
    /// The analysis of effect `id` at `sample_rate`; `Kind::None` (an empty slab) for an effect without one.
    pub(crate) fn new(id: u16, sample_rate: f32) -> Self {
        let (kind, len) = match id {
            27 => (Kind::Gate, 2),
            28 => (Kind::Limiter(History::new(sample_rate * 0.05)), HISTORY_LEN),
            // "one entry per 2205 samples (50 ms at 44.1 kHz)".
            36 => (
                Kind::Multiband(Box::new(std::array::from_fn(|_| History::new(2205.0)))),
                3 * HISTORY_LEN,
            ),
            34 => (Kind::Spacer(Box::default()), 2),
            44 => (Kind::Spreader(Box::new(Petal::new(0.9))), PETAL_LEN),
            43 => (Kind::WowFlutter(Trace::default()), 1 + TRACE_POINTS),
            46 => (Kind::Tremolo, 1),
            42 => (Kind::TapeStop(0.0), 1),
            41 => (Kind::Waveshaper, 2),
            24 => (Kind::Tuner(Box::new(Detector::new(sample_rate))), 2),
            25 => (Kind::Pitcher, 2),
            45 => (Kind::Pitcher2, 2),
            26 => (
                Kind::Analyzer(Box::new(Analyzer::new(sample_rate))),
                analyzer::LEN,
            ),
            _ => (Kind::None, 0),
        };
        let mut probe = Self {
            kind,
            slab: vec![0.0; len],
        };
        probe.reset();
        probe
    }

    /// Gives the Analyzer `setting`; `false` for another effect.
    pub(crate) fn set_analyzer(&mut self, setting: analyzer::Setting) -> bool {
        match &mut self.kind {
            Kind::Analyzer(analyzer) => {
                analyzer.set(&mut self.slab, setting);
                true
            }
            _ => false,
        }
    }

    /// What it publishes.
    pub(crate) fn slab(&self) -> &[f32] {
        &self.slab
    }

    /// Back to rest, as after a transport reset: empty histories, no correlation, no pitch.
    pub(crate) fn reset(&mut self) {
        self.slab.fill(0.0);
        match &mut self.kind {
            Kind::Gate => self.slab.fill(-60.0),
            Kind::Limiter(history) => history.reset(),
            Kind::Multiband(bands) => bands.iter_mut().for_each(History::reset),
            Kind::Spacer(pair) => **pair = Default::default(),
            Kind::Spreader(petal) => petal.reset(),
            Kind::WowFlutter(trace) => {
                *trace = Trace::default();
                self.slab[1..].fill(0.5);
            }
            Kind::TapeStop(theta) => *theta = 0.0,
            Kind::Tuner(detector) => detector.reset(),
            Kind::Pitcher | Kind::Pitcher2 => self.slab[1] = -1.0,
            Kind::Analyzer(analyzer) => analyzer.reset(&mut self.slab),
            Kind::None | Kind::Tremolo | Kind::Waveshaper => {}
        }
    }

    /// One frame: what reached the effect, what left it, its processor after the frame, and whether the transport
    /// plays.
    pub(crate) fn frame(
        &mut self,
        input: (f32, f32),
        output: (f32, f32),
        processor: &Processor,
        playing: bool,
    ) {
        let slab = &mut self.slab[..];
        match (&mut self.kind, processor) {
            (Kind::Limiter(history), Processor::LookaheadLimiter(limiter)) => {
                let gain = limiter.input_gain;
                let level = (input.0 * gain).abs().max((input.1 * gain).abs());
                history.frame(slab, level, peak(output), limiter.gain);
            }
            (Kind::Multiband(bands), Processor::Multiband(state)) => {
                for ((history, slab), band) in bands
                    .iter_mut()
                    .zip(slab.chunks_exact_mut(HISTORY_LEN))
                    .zip(state.band_levels())
                {
                    history.frame(slab, band[0], band[1], band[2]);
                }
            }
            (Kind::Spacer(pair), _) => {
                for (k, (frame, correlation)) in
                    [input, output].into_iter().zip(&mut **pair).enumerate()
                {
                    if let Some(c) = correlation.frame(frame) {
                        slab[k] = c;
                    }
                }
            }
            (Kind::Spreader(petal), _) => petal.frame(slab, output),
            (Kind::WowFlutter(trace), Processor::WowFlutter(state)) => {
                trace.frame(slab, state.trace)
            }
            (Kind::TapeStop(theta), Processor::TapeStop(state)) if playing => {
                *theta = (*theta - 0.005 * f64::from(state.playback_speed())).rem_euclid(360.0);
            }
            (Kind::Analyzer(analyzer), _) => analyzer.frame(slab, input),
            (Kind::Tuner(detector), _) => {
                if let Some(hz) = detector.frame(0.5 * (input.0 + input.1)) {
                    slab[0] = (slab[0] + 1.0) % UPDATES;
                    slab[1] = hz;
                }
            }
            _ => {}
        }
    }

    /// The end of a block: the values the effect holds in its state.
    pub(crate) fn block(&mut self, processor: &Processor) {
        let slab = &mut self.slab[..];
        match (&mut self.kind, processor) {
            (Kind::Gate, Processor::Gate(gate)) => slab.copy_from_slice(&gate.level_db),
            (Kind::Tremolo, Processor::Tremolo(tremolo)) => slab[0] = tremolo.phase[0] as f32,
            (Kind::TapeStop(theta), _) => slab[0] = *theta as f32,
            (Kind::Waveshaper, Processor::Waveshaper(state)) => {
                (slab[0], slab[1]) = state.effective_xy();
            }
            (Kind::Pitcher, Processor::Pitcher(state)) => {
                let hz = state.voiced_frequency();
                slab[0] = hz;
                slab[1] = target_class(state);
            }
            (Kind::Pitcher2, Processor::Pitcher2(state)) => {
                slab[0] = state.tracked_frequency();
                slab[1] = state.target_frequency();
            }
            _ => {}
        }
    }
}

/// The Pitcher's target note as a pitch class, or −1 without one.
fn target_class(state: &PitcherState) -> f32 {
    state.target_class().map_or(-1.0, |class| class as f32)
}

/// max(|L|, |R|).
fn peak((l, r): (f32, f32)) -> f32 {
    l.abs().max(r.abs())
}

/// The Tuner's update count wraps here, inside f32's exact integers.
const UPDATES: f32 = 1_048_576.0;

/// A history's slab: the next column to write, then the input, output and gain series of [`HISTORY`] entries each, as
/// [`history_level`]s.
const HISTORY_LEN: usize = 1 + 3 * HISTORY;

/// A scrolling level history: every `period` samples one entry of the input and output peaks over it and the
/// gain at its end.
struct History {
    period: f32,
    /// Samples into the current entry, and its peaks so far.
    count: f32,
    input: f32,
    output: f32,
}

impl History {
    fn new(period: f32) -> Self {
        Self {
            period: period.max(1.0),
            count: 0.0,
            input: 0.0,
            output: 0.0,
        }
    }

    fn reset(&mut self) {
        (self.count, self.input, self.output) = (0.0, 0.0, 0.0);
    }

    fn frame(&mut self, slab: &mut [f32], input: f32, output: f32, gain: f32) {
        self.input = self.input.max(input);
        self.output = self.output.max(output);
        self.count += 1.0;
        if self.count < self.period {
            return;
        }
        self.count -= self.period;
        let next = slab[0] as usize;
        for (series, value) in [self.input, self.output, gain].into_iter().enumerate() {
            slab[1 + series * HISTORY + next] = history_level(value);
        }
        slab[0] = ((next + 1) % HISTORY) as f32;
        (self.input, self.output) = (0.0, 0.0);
    }
}

/// The L/R correlation of each [`BLOCK`] samples, Σ LR / √(Σ L²·Σ R²); 0 for silence.
#[derive(Default)]
struct Correlation {
    count: usize,
    lr: f32,
    ll: f32,
    rr: f32,
}

/// Below this Σ L² (or Σ R²) over a block a channel is silent (about −90 dB RMS over 2048 samples).
const SILENT: f32 = 2e-6;

impl Correlation {
    /// Adds a frame; the block's correlation when it completes.
    fn frame(&mut self, (l, r): (f32, f32)) -> Option<f32> {
        self.lr += l * r;
        self.ll += l * l;
        self.rr += r * r;
        self.count += 1;
        if self.count < BLOCK {
            return None;
        }
        let c = if self.ll > SILENT && self.rr > SILENT {
            (self.lr / (self.ll * self.rr).sqrt()).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        *self = Self::default();
        Some(c)
    }
}

/// A vectorscope's slab: the balance, then the [`PETAL_BINS`] bin values.
const PETAL_LEN: usize = 1 + PETAL_BINS;

/// The vectorscope: per [`BLOCK`] samples, each bin's largest √((L² + R²)/2) and the RMS balance, smoothed.
struct Petal {
    /// The bins' old-value weight while falling (Spreader 0.9); rising it is 0.5.
    falling: f32,
    count: usize,
    bins: [f32; PETAL_BINS],
    squares: [f32; 2],
}

impl Petal {
    fn new(falling: f32) -> Self {
        Self {
            falling,
            count: 0,
            bins: [0.0; PETAL_BINS],
            squares: [0.0; 2],
        }
    }

    fn reset(&mut self) {
        *self = Self::new(self.falling);
    }

    fn frame(&mut self, slab: &mut [f32], (l, r): (f32, f32)) {
        // The direction from the right-hand baseline, mid up and side across: pure R at 45°, mono at 90°, pure L at
        // 135°. (L, R) and (−L, −R) are one direction, so a negative mid turns round, and anti-phase lands on the
        // outer bins (15° and 165°).
        let sign = if l + r < 0.0 { -1.0 } else { 1.0 };
        let angle = (sign * (l + r)).atan2(sign * (r - l)).to_degrees();
        let bin = ((angle - 15.0) / 15.0)
            .round()
            .clamp(0.0, (PETAL_BINS - 1) as f32) as usize;
        let level = ((l * l + r * r) / 2.0).sqrt().min(1.0);
        self.bins[bin] = self.bins[bin].max(level);
        self.squares[0] += l * l;
        self.squares[1] += r * r;
        self.count += 1;
        if self.count < BLOCK {
            return;
        }
        let [rms_l, rms_r] = self.squares.map(|s| (s / BLOCK as f32).sqrt());
        let balance = if rms_l + rms_r > 1e-6 {
            (rms_l - rms_r) / (rms_l + rms_r)
        } else {
            0.0
        };
        slab[0] = flush(0.6 * slab[0] + 0.4 * balance);
        for (shown, &new) in slab[1..].iter_mut().zip(&self.bins) {
            let weight = if new > *shown { 0.5 } else { self.falling };
            *shown = flush(weight * *shown + (1.0 - weight) * new);
        }
        let falling = self.falling;
        *self = Self::new(falling);
    }
}

/// The Wow & Flutter trace's slab: the next point to write, then [`TRACE_POINTS`] averages.
#[derive(Default)]
struct Trace {
    count: usize,
    sum: f32,
}

impl Trace {
    fn frame(&mut self, slab: &mut [f32], value: f32) {
        self.sum += value;
        self.count += 1;
        if self.count < TRACE_ENTRIES {
            return;
        }
        let next = slab[0] as usize;
        slab[1 + next] = self.sum / TRACE_ENTRIES as f32;
        slab[0] = ((next + 1) % TRACE_POINTS) as f32;
        *self = Self::default();
    }
}

/// The Tuner's pitch detector (the Pitcher's normalised autocorrelation over
/// 40–1500 Hz, run every [`Detector::HOP`] samples on the mono input decimated to about 12 kHz, taking the first
/// strong peak).
struct Detector {
    rate: f32,
    decimation: usize,
    phase: usize,
    ring: Vec<f32>,
    /// The ring slot the next sample goes to, and whether the ring has been round once.
    written: usize,
    filled: bool,
    /// The window and its lags for a reading, newest first: the ring unrolled so the sums run on plain slices (sized at
    /// build).
    linear: Vec<f32>,
    scores: Vec<f32>,
    frames: usize,
}

impl Detector {
    const HOP: usize = 512;
    const LOWEST: f32 = 40.0;
    const HIGHEST: f32 = 1500.0;
    /// Under this RMS over the window (−60 dB) there is no pitch.
    const QUIET: f32 = 0.001;
    /// The weakest normalised correlation that counts as a pitch.
    const CLEAR: f32 = 0.5;
    /// A peak this close to the highest one wins if it comes first.
    const KEY: f32 = 0.9;

    fn new(sample_rate: f32) -> Self {
        let decimation = ((sample_rate / 12_000.0).floor() as usize).max(1);
        let rate = sample_rate / decimation as f32;
        let max_lag = (rate / Self::LOWEST).ceil() as usize;
        Self {
            rate,
            decimation,
            phase: 0,
            ring: vec![0.0; 3 * max_lag + 1],
            written: 0,
            filled: false,
            linear: vec![0.0; 3 * max_lag],
            scores: vec![0.0; max_lag + 2],
            frames: 0,
        }
    }

    fn reset(&mut self) {
        self.ring.fill(0.0);
        (self.phase, self.written, self.filled, self.frames) = (0, 0, false, 0);
    }

    /// The sample at `back` decimated samples before the newest.
    #[cfg(test)]
    fn at(&self, back: usize) -> f32 {
        let len = self.ring.len();
        self.ring[(self.written + len - 1 - back % len) % len]
    }

    /// Adds a mono input sample; every [`Self::HOP`] samples, once the ring is full, the pitch in Hz (0: none).
    fn frame(&mut self, sample: f32) -> Option<f32> {
        self.phase += 1;
        if self.phase == self.decimation {
            self.phase = 0;
            self.ring[self.written] = sample;
            self.written = (self.written + 1) % self.ring.len();
            self.filled |= self.written == 0;
        }
        self.frames += 1;
        if self.frames < Self::HOP || !self.filled {
            return None;
        }
        self.frames = 0;
        Some(self.detect())
    }

    fn detect(&mut self) -> f32 {
        let min_lag = ((self.rate / Self::HIGHEST).floor() as usize).max(2);
        let max_lag = self.scores.len() - 2;
        let window = 2 * max_lag;
        let len = self.ring.len();
        for (back, slot) in self.linear.iter_mut().enumerate() {
            *slot = self.ring[(self.written + len - 1 - back) % len];
        }
        // The window's energy is the same for every lag.
        let x = &self.linear;
        let a2 = x[..window].iter().fold(0.0, |sum, a| sum + a * a);
        if a2 / (window as f32) < Self::QUIET * Self::QUIET {
            return 0.0;
        }
        self.scores.fill(0.0);
        for lag in min_lag..=max_lag {
            let (mut cross, mut b2) = (0.0, 0.0);
            for (a, b) in x[..window].iter().zip(&x[lag..lag + window]) {
                cross += a * b;
                b2 += b * b;
            }
            if a2 > 0.0 && b2 > 0.0 {
                self.scores[lag] = cross / (a2 * b2).sqrt();
            }
        }
        // The first peak nearly as high as the highest (McLeod's key maximum): a low tone's slow fall from lag 0 is no
        // peak, and its later periods are no higher than its first.
        let s = &self.scores;
        let peaks = || (min_lag + 1..max_lag).filter(|&l| s[l] > s[l - 1] && s[l] >= s[l + 1]);
        let top = peaks().map(|l| s[l]).fold(0.0, f32::max);
        if top < Self::CLEAR {
            return 0.0;
        }
        let Some(best) = peaks().find(|&l| s[l] >= Self::KEY * top) else {
            return 0.0;
        };
        let (previous, current, next) = (s[best - 1], s[best], s[best + 1]);
        let denominator = previous - 2.0 * current + next;
        let offset = if denominator.abs() > 1e-6 {
            (0.5 * (previous - next) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        self.rate / (best as f32 + offset)
    }
}

/// A note effect's slab (it runs ahead of playback; the engine fills this from its [`NoteEffectTrace`]): an
/// Arpeggiator's per engine running (1 or 0) and playhead in steps, a Note Echo's pending (1 or 0).
pub fn note_effect_len(trace: &NoteEffectTrace) -> usize {
    match trace {
        NoteEffectTrace::Arp { .. } => 4,
        NoteEffectTrace::Echo { .. } => 1,
    }
}

/// The playhead wraps here: 840 is a multiple of every pattern length (2 to 8 steps), so the step within any pattern
/// survives, inside f32's precision.
const STEPS: f64 = 840.0;

/// Writes `trace`'s state at song `beat` into `slab` ([`note_effect_len`] long): only while the transport `rolls` is
/// an arp running or an echo pending. No allocation: a binary search per clock.
pub fn note_effect_state(trace: &NoteEffectTrace, beat: f64, rolls: bool, slab: &mut [f32]) {
    match trace {
        NoteEffectTrace::Arp { clocks, .. } => {
            for (k, clock) in clocks.iter().enumerate() {
                let at = clock.partition_point(|mark| mark.beat <= beat);
                let mark = at.checked_sub(1).map(|i| clock[i]);
                let step = mark
                    .filter(|m| rolls && m.enabled && m.interval > 0.0)
                    .and_then(|m| {
                        m.origin
                            .map(|origin| ((beat - origin) / m.interval).rem_euclid(STEPS))
                    });
                slab[2 * k] = f32::from(u8::from(step.is_some()));
                slab[2 * k + 1] = step.unwrap_or(0.0) as f32;
            }
        }
        NoteEffectTrace::Echo { pending, .. } => {
            let at = pending.partition_point(|span| span.1 <= beat);
            let inside = pending.get(at).is_some_and(|span| span.0 <= beat);
            slab[0] = f32::from(u8::from(rolls && inside));
        }
    }
}

// ---- The views: each panel's reading of its effect's slab. `None` for a slab of another layout. ----

/// Gate: the detector's RMS levels L and R, dB (−60 at rest).
pub fn gate(slab: &[f32]) -> Option<[f32; 2]> {
    slab.try_into().ok()
}

/// A history display's series, each [`HISTORY`] [`history_level`]s oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryView {
    pub input: [f32; HISTORY],
    pub output: [f32; HISTORY],
    pub gain: [f32; HISTORY],
}

fn history_view(slab: &[f32]) -> HistoryView {
    let next = (slab[0] as usize) % HISTORY;
    let series = |k: usize| -> [f32; HISTORY] {
        let s = &slab[1 + k * HISTORY..1 + (k + 1) * HISTORY];
        std::array::from_fn(|i| s[(next + i) % HISTORY])
    };
    HistoryView {
        input: series(0),
        output: series(1),
        gain: series(2),
    }
}

/// Limiter: the 6.4 s history.
pub fn limiter(slab: &[f32]) -> Option<HistoryView> {
    (slab.len() == HISTORY_LEN).then(|| history_view(slab))
}

/// Multiband Compressor: band `band`'s history (0 Low, 1 Mid, 2 High).
pub fn multiband(slab: &[f32], band: usize) -> Option<HistoryView> {
    let at = band * HISTORY_LEN;
    (slab.len() == 3 * HISTORY_LEN && band < 3).then(|| history_view(&slab[at..at + HISTORY_LEN]))
}

/// Spacer: the input's and the output's correlation, −1…1.
pub fn spacer(slab: &[f32]) -> Option<[f32; 2]> {
    slab.try_into().ok()
}

/// A vectorscope: the smoothed balance (−1 right … 1 left) and bins (0…1, 15° to 165°).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PetalView {
    pub balance: f32,
    pub bins: [f32; PETAL_BINS],
}

/// Spreader: its vectorscope.
pub fn spreader(slab: &[f32]) -> Option<PetalView> {
    (slab.len() == PETAL_LEN).then(|| PetalView {
        balance: slab[0],
        bins: std::array::from_fn(|i| slab[1 + i]),
    })
}

/// Wow & Flutter: the trace's points oldest first, each a fraction of the Y span (0.5 at rest).
pub fn wow_flutter(slab: &[f32]) -> Option<Vec<f32>> {
    (slab.len() == 1 + TRACE_POINTS).then(|| {
        let next = (slab[0] as usize) % TRACE_POINTS;
        (0..TRACE_POINTS)
            .map(|i| slab[1 + (next + i) % TRACE_POINTS])
            .collect()
    })
}

/// Tremolo: the left oscillator's phase, 0…1.
pub fn tremolo(slab: &[f32]) -> Option<f32> {
    match slab {
        &[phase] => Some(phase),
        _ => None,
    }
}

/// Tape Stop: the spools' angle θ, degrees in [0, 360).
pub fn tape_stop(slab: &[f32]) -> Option<f32> {
    match slab {
        &[theta] => Some(theta),
        _ => None,
    }
}

/// Waveshaper: the effective X and Y.
pub fn waveshaper(slab: &[f32]) -> Option<(f32, f32)> {
    match slab {
        &[x, y] => Some((x, y)),
        _ => None,
    }
}

/// Tuner: the detector's update count (it changes with each reading) and its last reading, Hz (0: no pitch).
pub fn tuner(slab: &[f32]) -> Option<(u32, f32)> {
    match slab {
        &[updates, hz] => Some((updates as u32, hz)),
        _ => None,
    }
}

/// Pitcher: the detected pitch, Hz (0: none), and the target note's pitch class (C = 0), if any.
pub fn pitcher(slab: &[f32]) -> Option<(f32, Option<usize>)> {
    match slab {
        &[hz, class] => Some((hz, (class >= 0.0).then_some(class as usize))),
        _ => None,
    }
}

/// Pitcher 2: the detected and target pitches, Hz (0: none).
pub fn pitcher2(slab: &[f32]) -> Option<(f32, f32)> {
    match slab {
        &[hz, target] => Some((hz, target)),
        _ => None,
    }
}

/// Arpeggiator: each engine's playhead in steps while it runs (the step k sounds at k; mod 840), else `None`.
pub fn arpeggiator(slab: &[f32]) -> Option<[Option<f32>; 2]> {
    match slab {
        &[run0, step0, run1, step1] => {
            Some([(run0, step0), (run1, step1)].map(|(run, step)| (run > 0.5).then_some(step)))
        }
        _ => None,
    }
}

/// Note Echo: echoes are pending.
pub fn note_echo(slab: &[f32]) -> Option<bool> {
    match slab {
        &[pending] => Some(pending > 0.5),
        _ => None,
    }
}

/// The Analyzer's data ([`analyzer`]), each history oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzerView {
    /// The smoothed spectrum, dB per bin k at k·rate/2048 Hz ([`analyzer::BINS`]).
    pub spectrum: Vec<f32>,
    /// The last second as (min, max) per quarter millisecond ([`analyzer::SCOPE_PER_MS`]).
    pub scope: Vec<(f32, f32)>,
    /// Per meter (Peak, True Peak, RMS, LUFS 0.4 s, LUFS 3 s), dB or LUFS per 50 ms entry ([`analyzer::FLOOR`]: none).
    pub loudness: [Vec<f32>; analyzer::METERS],
    /// Each meter's running maximum.
    pub maxima: [f32; analyzer::METERS],
    /// Integrated loudness (LUFS) and loudness range (LU), once there is any.
    pub integrated: Option<f32>,
    pub range: Option<f32>,
    pub petal: PetalView,
    /// The interferometer: one L/R correlation per 2048 samples.
    pub correlations: Vec<f32>,
    /// The pitch track: Hz per 50 ms entry, 0 for none.
    pub pitch: Vec<f32>,
}

/// Analyzer: its views' data.
pub fn analyzer(slab: &[f32]) -> Option<AnalyzerView> {
    use analyzer::*;
    if slab.len() != LEN {
        return None;
    }
    let ring = |at: usize, len: usize| -> Vec<f32> {
        let next = (slab[at] as usize) % len;
        (0..len).map(|i| slab[at + 1 + (next + i) % len]).collect()
    };
    let next = (slab[SCOPE_AT] as usize) % SCOPE;
    let scope = (0..SCOPE)
        .map(|i| {
            let k = (next + i) % SCOPE;
            (slab[SCOPE_AT + 1 + 2 * k], slab[SCOPE_AT + 2 + 2 * k])
        })
        .collect();
    let next = (slab[LOUDNESS_AT] as usize) % LOUDNESS;
    let loudness = std::array::from_fn(|m| {
        let at = LOUDNESS_AT + 1 + m * LOUDNESS;
        (0..LOUDNESS)
            .map(|i| slab[at + (next + i) % LOUDNESS])
            .collect()
    });
    let summary = |i: usize| Some(slab[SUMMARY_AT + METERS + i]).filter(|v| v.is_finite());
    Some(AnalyzerView {
        spectrum: slab[SPECTRUM_AT..SPECTRUM_AT + BINS].to_vec(),
        scope,
        loudness,
        maxima: std::array::from_fn(|m| slab[SUMMARY_AT + m]),
        integrated: summary(0),
        range: summary(1),
        petal: PetalView {
            balance: slab[PETAL_AT],
            bins: std::array::from_fn(|i| slab[PETAL_AT + 1 + i]),
        },
        correlations: ring(CORRELATION_AT, CORRELATIONS),
        pitch: ring(PITCH_AT, PITCH),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioEffect, ProcessContext};

    const RATE: f32 = 48_000.0;

    /// Effect `id` with `params` set, at 48 kHz.
    fn effect(id: u16, params: &[(usize, f32)]) -> AudioEffect {
        let mut effect = AudioEffect::new(id, RATE).unwrap();
        for &(index, value) in params {
            effect.set_parameter(index, value).unwrap();
        }
        effect
    }

    /// Runs `frames` frames of `signal(frame) -> (L, R)` through `effect` in 480-frame blocks, transport `playing`.
    fn run(
        effect: &mut AudioEffect,
        frames: usize,
        playing: bool,
        signal: impl Fn(usize) -> (f32, f32),
    ) {
        let mut done = 0;
        while done < frames {
            let n = 480.min(frames - done);
            let (mut left, mut right): (Vec<f32>, Vec<f32>) = (done..done + n).map(&signal).unzip();
            let context = ProcessContext {
                sample_rate: RATE,
                tempo_bpm: 120.0,
                start_beat: 0.0,
                playing,
            };
            effect.process(&mut left, &mut right, context).unwrap();
            done += n;
        }
    }

    fn sine(hz: f32, amplitude: f32) -> impl Fn(usize) -> (f32, f32) {
        move |i| {
            let v = amplitude * (std::f32::consts::TAU * hz * i as f32 / RATE).sin();
            (v, v)
        }
    }

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn effects_without_an_analysis_publish_nothing() {
        for id in [3, 8, 9, 20] {
            assert!(
                AudioEffect::new(id, RATE).unwrap().analysis().is_empty(),
                "{id}"
            );
        }
    }

    #[test]
    fn the_input_peak_is_what_reached_the_module_before_it_ran() {
        // A Compressor's makeup changes the output; the input peak is the louder input channel's.
        let mut compressor = effect(8, &[(5, 1.0)]);
        run(&mut compressor, 480, true, |_| (0.25, -0.5));
        let meter = compressor.meter();
        assert_eq!(meter.input_peak, 0.5);
        assert!(meter.peak_right > 0.5, "{meter:?}");
    }

    #[test]
    fn the_gate_publishes_its_detector_rms_levels() {
        let mut gate = effect(27, &[]);
        assert_eq!(super::gate(gate.analysis()).unwrap(), [-60.0, -60.0]);
        // A full-scale sine is −3 dB RMS; the right channel silent stays at −60.
        run(&mut gate, 4800, true, |i| (sine(1000.0, 1.0)(i).0, 0.0));
        let [left, right] = super::gate(gate.analysis()).unwrap();
        // 256-sample RMS windows hold 5⅓ periods: within 0.1 dB.
        assert!(near(left, -3.01, 0.1), "{left}");
        assert_eq!(right, -60.0);
    }

    #[test]
    fn the_limiter_history_adds_an_entry_every_50_ms() {
        // In Gain +6 dB (p1 0.625), threshold −24 dB (p4 0): a −6 dB tone reaches 0 dB in, and is limited to −24 dB.
        let mut limiter = effect(28, &[(1, 0.625), (4, 0.0)]);
        assert_eq!(
            super::limiter(limiter.analysis()).unwrap().input,
            [0.0; HISTORY]
        );
        run(&mut limiter, 3 * 2400 + 100, true, sine(440.0, 0.5));
        let history = super::limiter(limiter.analysis()).unwrap();
        // Three entries, newest last; older columns still empty.
        assert_eq!(history.input[..HISTORY - 3], [0.0; HISTORY - 3]);
        let newest = HISTORY - 1;
        assert!(
            near(history.input[newest], 60.0 / 66.0, 0.01),
            "{}",
            history.input[newest]
        );
        assert!(
            near(history.output[newest], 36.0 / 66.0, 0.02),
            "{}",
            history.output[newest]
        );
        // About −24 dB of gain on the 66 dB scale.
        assert!(
            near(history.gain[newest], 36.0 / 66.0, 0.03),
            "{}",
            history.gain[newest]
        );
        // 128 entries on, the oldest has scrolled off: every column is full.
        run(&mut limiter, HISTORY * 2400, true, sine(440.0, 0.5));
        assert!(
            super::limiter(limiter.analysis())
                .unwrap()
                .input
                .iter()
                .all(|&f| f > 0.8)
        );
        // A transport reset empties it.
        limiter.reset();
        assert_eq!(
            super::limiter(limiter.analysis()).unwrap().gain,
            [0.0; HISTORY]
        );
    }

    #[test]
    fn each_multiband_band_keeps_its_own_history_every_2205_samples() {
        // A 100 Hz tone sits in the low band (crossovers at their defaults): the low history fills, the high stays
        // nearly silent.
        let mut multiband = effect(36, &[]);
        run(&mut multiband, 2 * 2205 + 10, true, sine(100.0, 0.5));
        let slab = multiband.analysis();
        let (low, high) = (
            super::multiband(slab, 0).unwrap(),
            super::multiband(slab, 2).unwrap(),
        );
        let newest = HISTORY - 1;
        assert!(
            low.input[newest] > history_level(0.3),
            "{}",
            low.input[newest]
        );
        assert!(low.output[newest] > 0.0);
        assert!(
            high.input[newest] < low.input[newest] - 0.2,
            "{}",
            high.input[newest]
        );
        // Two entries so far: the third column from the right is empty.
        assert_eq!(low.input[HISTORY - 3], 0.0);
        assert!(super::multiband(slab, 3).is_none());
    }

    #[test]
    fn the_spacer_correlates_input_and_output_per_block() {
        // Neutral settings pass the signal: mono reads +1 on both, anti-phase −1, silence 0.
        for (signal, want) in [((0.5, 0.5), 1.0), ((0.5, -0.5), -1.0), ((0.0, 0.0), 0.0)] {
            let mut spacer = effect(34, &[]);
            run(&mut spacer, 2 * BLOCK, true, |i| {
                let s = (i as f32 * 0.01).sin();
                (signal.0 * s, signal.1 * s)
            });
            let [input, _] = super::spacer(spacer.analysis()).unwrap();
            assert!(near(input, want, 1e-3), "{signal:?}: {input}");
        }
        // L/R inversion (p4 = 1/3, L inverted) turns a mono input to anti-phase at the output.
        let mut spacer = effect(34, &[(4, 1.0 / 3.0)]);
        run(&mut spacer, 2 * BLOCK, true, |i| {
            ((i as f32 * 0.01).sin(), (i as f32 * 0.01).sin())
        });
        let [input, output] = super::spacer(spacer.analysis()).unwrap();
        assert!(
            near(input, 1.0, 1e-3) && near(output, -1.0, 1e-3),
            "{input} {output}"
        );
    }

    #[test]
    fn the_spreader_vectorscope_bins_directions_and_balances() {
        // The vectorscope shows what leaves the module: bypassed (p0 0), the input. A pure-left signal lands on the
        // 135° bin (8), and the balance leans left.
        let mut spreader = effect(44, &[(0, 0.0)]);
        run(&mut spreader, 8 * BLOCK, true, |i| {
            (0.5 * (i as f32 * 0.05).sin(), 0.0)
        });
        let petal = super::spreader(spreader.analysis()).unwrap();
        let loudest = (0..PETAL_BINS)
            .max_by(|&a, &b| petal.bins[a].total_cmp(&petal.bins[b]))
            .unwrap();
        assert_eq!(loudest, 8, "{petal:?}");
        assert!(petal.balance > 0.9, "{petal:?}");
        // Mono lands on 90° (bin 5).
        let mut spreader = effect(44, &[(0, 0.0)]);
        run(&mut spreader, 8 * BLOCK, true, |i| {
            let v = 0.5 * (i as f32 * 0.05).sin();
            (v, v)
        });
        let petal = super::spreader(spreader.analysis()).unwrap();
        let loudest = (0..PETAL_BINS)
            .max_by(|&a, &b| petal.bins[a].total_cmp(&petal.bins[b]))
            .unwrap();
        assert_eq!(loudest, 5, "{petal:?}");
        assert!(near(petal.balance, 0.0, 0.05), "{petal:?}");
        // Silence decays the bins at 0.9 per block.
        let before = petal.bins[5];
        run(&mut spreader, BLOCK, true, |_| (0.0, 0.0));
        let after = super::spreader(spreader.analysis()).unwrap().bins[5];
        assert!(near(after, 0.9 * before, 1e-4), "{before} {after}");
    }

    #[test]
    fn the_spreader_vectorscope_shows_the_width_it_makes() {
        // A mono input sits on the 90° bin alone, as Width 0 (p1 0) leaves it. At the widest Width (p1 1, 2v = 2) the
        // output turns partly anti-phase, and the vectorscope shows the output: the outer bins fill.
        let mono = |i: usize| {
            let v = 0.3 * (i as f32 * 0.05).sin() + 0.2 * (i as f32 * 0.31).sin();
            (v, v)
        };
        let petal = |width: f32| {
            let mut spreader = effect(44, &[(1, width)]);
            run(&mut spreader, 16 * BLOCK, true, mono);
            super::spreader(spreader.analysis()).unwrap()
        };
        let narrow = petal(0.0);
        assert!(narrow.bins[5] > 0.4, "{narrow:?}");
        assert!(
            (0..PETAL_BINS).all(|k| k == 5 || narrow.bins[k] < 1e-3),
            "{narrow:?}"
        );
        let wide = petal(1.0);
        assert!(wide.bins[0] > 0.8 && wide.bins[10] > 0.8, "{wide:?}");
        assert!(
            wide.bins[0] > wide.bins[5] && wide.bins[10] > wide.bins[5],
            "{wide:?}"
        );
    }

    #[test]
    fn the_wow_flutter_trace_rests_at_the_midpoint_and_wobbles_with_wow() {
        let mut still = effect(43, &[(3, 0.0), (6, 0.0)]);
        assert_eq!(
            super::wow_flutter(still.analysis()).unwrap(),
            vec![0.5; TRACE_POINTS]
        );
        run(&mut still, 48_000, true, |_| (0.0, 0.0));
        assert!(
            super::wow_flutter(still.analysis())
                .unwrap()
                .iter()
                .all(|&v| near(v, 0.5, 1e-4))
        );
        // Full wow at 1 Hz (p1 0.18): a second of points swings about the middle.
        let mut wobbly = effect(43, &[(1, 0.18), (3, 1.0), (6, 0.0)]);
        run(&mut wobbly, 2 * 48_000, true, |_| (0.0, 0.0));
        let points = super::wow_flutter(wobbly.analysis()).unwrap();
        let recent = &points[TRACE_POINTS - 240..];
        let (lo, hi) = recent
            .iter()
            .fold((1.0f32, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(lo < 0.3 && hi > 0.7 && lo >= 0.0 && hi <= 1.0, "{lo} {hi}");
    }

    #[test]
    fn the_tremolo_publishes_its_left_phase() {
        // Rate p1 = 0.5: 0.1 + 19.9·0.5 = 10.05 Hz; after 0.1 s the phase has turned 1.005 times.
        let mut tremolo = effect(46, &[(1, 0.5)]);
        run(&mut tremolo, 4800, false, |_| (0.1, 0.1));
        let phase = super::tremolo(tremolo.analysis()).unwrap();
        assert!(near(phase, 0.005, 1e-3), "{phase}");
    }

    #[test]
    fn the_tape_stop_spools_turn_with_the_tape_while_playing() {
        // Playing tape (rate 1): −0.005° a sample, so −240° over 48 000 samples, wrapped to 120°.
        let mut tape = effect(42, &[]);
        run(&mut tape, 48_000, true, |_| (0.1, 0.1));
        let theta = super::tape_stop(tape.analysis()).unwrap();
        assert!(near(theta, 120.0, 0.01), "{theta}");
        // Stopped transport: the spools stand.
        run(&mut tape, 48_000, false, |_| (0.1, 0.1));
        assert!(near(
            super::tape_stop(tape.analysis()).unwrap(),
            theta,
            1e-4
        ));
    }

    #[test]
    fn the_waveshaper_publishes_its_effective_x_and_y() {
        let mut shaper = effect(41, &[(3, 0.25), (4, 0.75)]);
        run(&mut shaper, 48_000, true, |_| (0.1, 0.1));
        let (x, y) = super::waveshaper(shaper.analysis()).unwrap();
        assert!(near(x, 0.25, 1e-3) && near(y, 0.75, 1e-3), "{x} {y}");
    }

    #[test]
    fn the_tuner_detects_a_tone_and_reports_silence() {
        let mut tuner = effect(24, &[]);
        assert_eq!(super::tuner(tuner.analysis()).unwrap(), (0, 0.0));
        run(&mut tuner, 24_000, true, sine(220.0, 0.5));
        let (updates, hz) = super::tuner(tuner.analysis()).unwrap();
        assert!(updates > 10, "{updates}");
        assert!(near(hz, 220.0, 1.0), "{hz}");
        // A low E and a high A.
        for want in [82.41, 880.0] {
            let mut tuner = effect(24, &[]);
            run(&mut tuner, 24_000, true, sine(want, 0.5));
            let hz = super::tuner(tuner.analysis()).unwrap().1;
            assert!(near(hz, want, want * 0.005), "{want}: {hz}");
        }
        run(&mut tuner, 24_000, true, |_| (0.0, 0.0));
        let (later, hz) = super::tuner(tuner.analysis()).unwrap();
        assert!(later > updates);
        assert_eq!(hz, 0.0);
    }

    #[test]
    fn the_analyzer_measures_its_input_for_every_view() {
        // A 1 kHz sine at −6 dBFS on both channels, for 4 s (the short-term window is 3 s).
        let mut analyzer = effect(26, &[]);
        let rest = super::analyzer(analyzer.analysis()).unwrap();
        assert!(rest.spectrum.iter().all(|&v| v == analyzer::FLOOR));
        assert_eq!((rest.integrated, rest.range), (None, None));
        run(&mut analyzer, 4 * 48_000, true, sine(1000.0, 0.5));
        let view = super::analyzer(analyzer.analysis()).unwrap();
        // Spectrum: its peak on bin 42 or 43 (1 kHz is 42.67 bins), about −6 dB; far bins down below −60.
        let (bin, level) =
            (view.spectrum.iter().enumerate()).fold((0, f32::MIN), |best, (k, &v)| {
                if v > best.1 { (k, v) } else { best }
            });
        assert!((42..=43).contains(&bin), "{bin}");
        assert!((-8.0..=-5.5).contains(&level), "{level}");
        assert!(view.spectrum[400] < -60.0, "{}", view.spectrum[400]);
        // Oscilloscope: the last 10 ms swing ±0.5.
        let recent = &view.scope[analyzer::SCOPE - 10 * analyzer::SCOPE_PER_MS..];
        let low = recent.iter().map(|b| b.0).fold(f32::MAX, f32::min);
        let high = recent.iter().map(|b| b.1).fold(f32::MIN, f32::max);
        assert!(
            near(low, -0.5, 0.01) && near(high, 0.5, 0.01),
            "{low} {high}"
        );
        // Loudness, the newest entry: Peak −6.02 dB, True Peak at least that, RMS −9.03 dB, and both LUFS about −6.
        let newest = view.loudness.each_ref().map(|m| m[analyzer::LOUDNESS - 1]);
        assert!(near(newest[0], -6.02, 0.05), "{newest:?}");
        assert!(
            newest[1] >= newest[0] - 1e-3 && newest[1] < newest[0] + 0.5,
            "{newest:?}"
        );
        assert!(near(newest[2], -9.03, 0.05), "{newest:?}");
        assert!(
            (-7.0..=-5.0).contains(&newest[3]) && near(newest[4], newest[3], 0.1),
            "{newest:?}"
        );
        assert!(near(view.maxima[0], -6.02, 0.05));
        let integrated = view.integrated.expect("integrated");
        assert!(near(integrated, newest[3], 0.2), "{integrated}");
        // The range counts the short-term window's ramp in from silence over the first 3 s.
        assert!((0.0..20.0).contains(&view.range.expect("range")));
        // Vectorscope: mono, the 90° bin; the interferometer's newest +1.
        let loudest =
            (0..PETAL_BINS).max_by(|&a, &b| view.petal.bins[a].total_cmp(&view.petal.bins[b]));
        assert_eq!(loudest, Some(5));
        assert!(near(*view.correlations.last().unwrap(), 1.0, 1e-3));
        // A transport reset rests it.
        analyzer.reset();
        assert_eq!(
            super::analyzer(analyzer.analysis()).unwrap().integrated,
            None
        );
    }

    /// The Analyzer's spectrum peak near 1 kHz (bins 41–44), dB.
    fn peak_1k(analyzer: &AudioEffect) -> f32 {
        let view = super::analyzer(analyzer.analysis()).unwrap();
        view.spectrum[41..=44]
            .iter()
            .copied()
            .fold(f32::MIN, f32::max)
    }

    #[test]
    fn the_analyzer_channel_picks_the_source_and_input_gain_scales_it() {
        use analyzer::Setting;
        // A −6 dBFS 1 kHz tone on the left only. L + R ((L + R)/2) reads −12 dB, Left −6 dB, Right nothing; the
        // loudness Peak follows the source too.
        let left_only = |i| (sine(1000.0, 0.5)(i).0, 0.0);
        for (channel, want) in [(2, -12.04), (0, -6.02), (1, analyzer::FLOOR)] {
            let mut analyzer = effect(26, &[]);
            analyzer.set_analyzer(Setting::Channel(channel)).unwrap();
            run(&mut analyzer, 48_000, true, left_only);
            let peak = peak_1k(&analyzer);
            assert!(
                near(peak, want, 1.5) || peak < -100.0 && want < -100.0,
                "{channel}: {peak}"
            );
            let loudness = super::analyzer(analyzer.analysis()).unwrap().loudness;
            let newest = loudness[0][analyzer::LOUDNESS - 1];
            assert!(near(newest, want, 0.05), "{channel}: Peak {newest}");
        }
        // The vectorscope keeps both channels: pure left lands on the 135° bin whatever the source.
        let mut analyzer = effect(26, &[]);
        analyzer.set_analyzer(Setting::Channel(1)).unwrap();
        run(&mut analyzer, 8 * BLOCK, true, left_only);
        let petal = super::analyzer(analyzer.analysis()).unwrap().petal;
        let loudest = (0..PETAL_BINS).max_by(|&a, &b| petal.bins[a].total_cmp(&petal.bins[b]));
        assert_eq!(loudest, Some(8), "{petal:?}");
        // Input Gain ×10 lifts every view by 20 dB.
        let mut analyzer = effect(26, &[]);
        analyzer.set_analyzer(Setting::InputGain(10.0)).unwrap();
        run(&mut analyzer, 48_000, true, sine(1000.0, 0.05));
        assert!(
            near(peak_1k(&analyzer), -6.02, 1.5),
            "{}",
            peak_1k(&analyzer)
        );
        let view = super::analyzer(analyzer.analysis()).unwrap();
        assert!(near(view.loudness[0][analyzer::LOUDNESS - 1], -6.02, 0.05));
        // Not an Analyzer: an error.
        assert!(
            effect(44, &[])
                .set_analyzer(Setting::DcBlock(true))
                .is_err()
        );
    }

    #[test]
    fn the_analyzer_dc_block_takes_the_offset_out() {
        use analyzer::Setting;
        // A 0.5 offset under a quiet 1 kHz tone: without the block bin 0 holds it; with it, bin 0 is far down while the
        // tone stays.
        let offset = |i| {
            let v = 0.5 + sine(1000.0, 0.05)(i).0;
            (v, v)
        };
        let dc = |on: bool| {
            let mut analyzer = effect(26, &[]);
            analyzer.set_analyzer(Setting::DcBlock(on)).unwrap();
            run(&mut analyzer, 48_000, true, offset);
            let view = super::analyzer(analyzer.analysis()).unwrap();
            (view.spectrum[0], peak_1k(&analyzer))
        };
        let ((off, tone_off), (on, tone_on)) = (dc(false), dc(true));
        assert!(off > -10.0, "{off}");
        assert!(on < off - 40.0, "{on}");
        assert!(near(tone_on, tone_off, 0.5), "{tone_on} {tone_off}");
    }

    #[test]
    fn a_spectrum_bin_decays_after_the_tone_stops_at_the_reactivity() {
        use analyzer::Setting;
        // Falling, a bin keeps `weight` of its old value per hop (1024 samples): Slow 0.999, Medium 0.95, Fast 0.7. Two
        // hops after the tone the window is all silence, which reads the −120 dB floor.
        for (reactivity, weight) in [(0, 0.999f32), (1, 0.95), (2, 0.7)] {
            let mut analyzer = effect(26, &[]);
            analyzer
                .set_analyzer(Setting::Reactivity(reactivity))
                .unwrap();
            run(&mut analyzer, 40 * 1024, true, sine(1000.0, 0.5));
            let on = peak_1k(&analyzer);
            run(&mut analyzer, 1024, true, |_| (0.0, 0.0));
            let half = super::analyzer(analyzer.analysis()).unwrap().spectrum;
            run(&mut analyzer, 1024, true, |_| (0.0, 0.0));
            let after = super::analyzer(analyzer.analysis()).unwrap().spectrum;
            for bin in 41..=44 {
                let want = weight * half[bin] + (1.0 - weight) * analyzer::FLOOR;
                assert!(
                    near(after[bin], want, 1e-3),
                    "{reactivity} bin {bin}: {} {want}",
                    after[bin]
                );
            }
            assert!(peak_1k(&analyzer) < on, "{reactivity}");
        }
    }

    #[test]
    fn the_loudness_mode_resets_the_meter() {
        use analyzer::Setting;
        let mut analyzer = effect(26, &[]);
        run(&mut analyzer, 48_000, true, sine(1000.0, 0.5));
        let view = super::analyzer(analyzer.analysis()).unwrap();
        assert!(view.maxima[0] > -7.0 && view.integrated.is_some());
        // The same mode again changes nothing; another clears the histories, maxima and summary.
        analyzer.set_analyzer(Setting::LoudnessMode(0)).unwrap();
        assert_eq!(super::analyzer(analyzer.analysis()).unwrap(), view);
        analyzer.set_analyzer(Setting::LoudnessMode(4)).unwrap();
        let reset = super::analyzer(analyzer.analysis()).unwrap();
        assert_eq!(reset.maxima, [analyzer::FLOOR; analyzer::METERS]);
        assert_eq!((reset.integrated, reset.range), (None, None));
        assert!(
            reset
                .loudness
                .iter()
                .flatten()
                .all(|&v| v == analyzer::FLOOR)
        );
        // The other views keep theirs.
        assert_eq!((reset.spectrum, reset.pitch), (view.spectrum, view.pitch));
        // And it measures again.
        run(&mut analyzer, 4800, true, sine(1000.0, 0.5));
        let again = super::analyzer(analyzer.analysis()).unwrap();
        assert!(near(again.maxima[0], -6.02, 0.05), "{:?}", again.maxima);
    }

    /// The detector's reading as it was first written: each lag's sums read the ring through [`Detector::at`].
    fn detect_through_the_ring(d: &Detector) -> f32 {
        let min_lag = ((d.rate / Detector::HIGHEST).floor() as usize).max(2);
        let max_lag = d.scores.len() - 2;
        let window = 2 * max_lag;
        let energy: f32 = (0..window).map(|i| d.at(i) * d.at(i)).sum();
        if energy / (window as f32) < Detector::QUIET * Detector::QUIET {
            return 0.0;
        }
        let mut s = vec![0.0f32; d.scores.len()];
        for (lag, score) in s.iter_mut().enumerate().take(max_lag + 1).skip(min_lag) {
            let (mut cross, mut a2, mut b2) = (0.0, 0.0, 0.0);
            for i in 0..window {
                let (a, b) = (d.at(i), d.at(i + lag));
                cross += a * b;
                a2 += a * a;
                b2 += b * b;
            }
            if a2 > 0.0 && b2 > 0.0 {
                *score = cross / (a2 * b2).sqrt();
            }
        }
        let peaks = || (min_lag + 1..max_lag).filter(|&l| s[l] > s[l - 1] && s[l] >= s[l + 1]);
        let top = peaks().map(|l| s[l]).fold(0.0, f32::max);
        if top < Detector::CLEAR {
            return 0.0;
        }
        let Some(best) = peaks().find(|&l| s[l] >= Detector::KEY * top) else {
            return 0.0;
        };
        let (previous, current, next) = (s[best - 1], s[best], s[best + 1]);
        let denominator = previous - 2.0 * current + next;
        let offset = if denominator.abs() > 1e-6 {
            (0.5 * (previous - next) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        d.rate / (best as f32 + offset)
    }

    #[test]
    fn the_pitch_detector_reads_exactly_as_through_its_ring() {
        // Tones, a glide, noise and silence, at two rates: every reading is bit for bit the ring-indexed one.
        let mut seed = 1u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for rate in [48_000.0, 44_100.0] {
            let mut detector = Detector::new(rate);
            let (mut readings, mut voiced) = (0, 0);
            for i in 0..(2.5 * rate) as usize {
                let t = i as f32 / rate;
                let hz = 110.0 * (1.0 + t);
                let sample = match i * 5 / (2.5 * rate) as usize {
                    0 => 0.5 * (std::f32::consts::TAU * 220.0 * t).sin(),
                    1 => 0.4 * (std::f32::consts::TAU * hz * t).sin() + 0.05 * noise(),
                    2 => 0.3 * noise(),
                    3 => 0.0,
                    _ => 0.3 * (std::f32::consts::TAU * 82.41 * t).sin(),
                };
                // A reading leaves the ring as it read it.
                if let Some(got) = detector.frame(sample) {
                    let want = detect_through_the_ring(&detector);
                    assert_eq!(got.to_bits(), want.to_bits(), "{rate} at {i}: {got} {want}");
                    readings += 1;
                    voiced += usize::from(got > 0.0);
                }
            }
            assert!(readings > 100 && voiced > 50, "{readings} {voiced}");
        }
    }

    #[test]
    fn the_interferometer_holds_2_s_at_192_khz() {
        let blocks = (2.0 * 192_000.0 / BLOCK as f32).ceil() as usize;
        assert!(
            analyzer::CORRELATIONS >= blocks,
            "{}",
            analyzer::CORRELATIONS
        );
    }

    #[test]
    fn the_analyzer_tracks_pitch_every_50_ms() {
        let mut analyzer = effect(26, &[]);
        run(&mut analyzer, 48_000, true, sine(220.0, 0.5));
        let pitch = super::analyzer(analyzer.analysis()).unwrap().pitch;
        assert!(
            near(pitch[analyzer::PITCH - 1], 220.0, 1.0),
            "{:?}",
            &pitch[analyzer::PITCH - 5..]
        );
        // Before the detector's first reading, none.
        assert_eq!(pitch[0], 0.0);
    }

    #[test]
    fn the_pitchers_publish_their_tracked_pitch_and_target() {
        // Pitcher: 220 Hz (A3) in C minor, the default scale: A is not in it, so it snaps to G# (8) or B♭ (10); the
        // nearer is either at a semitone, the tie going to the lower.
        let mut pitcher = effect(25, &[]);
        run(&mut pitcher, 24_000, true, sine(220.0, 0.5));
        let (hz, target) = super::pitcher(pitcher.analysis()).unwrap();
        assert!(near(hz, 220.0, 2.0), "{hz}");
        assert!(matches!(target, Some(8 | 10)), "{target:?}");
        run(&mut pitcher, 24_000, true, |_| (0.0, 0.0));
        assert_eq!(super::pitcher(pitcher.analysis()).unwrap(), (0.0, None));
        // Bypass snapping: no target.
        let mut bypassed = effect(25, &[(22, 1.0)]);
        run(&mut bypassed, 24_000, true, sine(220.0, 0.5));
        assert_eq!(super::pitcher(bypassed.analysis()).unwrap().1, None);
        // Pitcher 2.
        let mut pitcher2 = effect(45, &[]);
        run(&mut pitcher2, 24_000, true, sine(220.0, 0.5));
        let (hz, target) = super::pitcher2(pitcher2.analysis()).unwrap();
        assert!(near(hz, 220.0, 2.0), "{hz}");
        assert!(target > 0.0);
        run(&mut pitcher2, 24_000, true, |_| (0.0, 0.0));
        assert_eq!(super::pitcher2(pitcher2.analysis()).unwrap(), (0.0, 0.0));
    }
}
