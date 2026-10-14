//! The Analyzer's (ID 26) analysis: the spectrum, the oscilloscope's samples, the
//! loudness history, the vectorscope with its interferometer, and the pitch track, all of the module's input.
//!
//! The panel's `Channel`, `Reactivity`, `DC Block`, `Input Gain` and LOUDNESS `Mode` arrive as [`Setting`]s (the engine's
//! `SetSetting`); a new module analyses `L + R` at ×1, DC Block off, `Medium`.
//!
//! Details:
//! - DC Block and Input Gain condition both channels, so they reach the vectorscope too; `Channel` picks the source of the
//!   other views.
//! - The DC blocker is a one-pole high pass at 10 Hz, y = x − x₁ + p·y₁.
//! - The loudness meters measure the source: LUFS takes it as both channels of BS.1770's sum (a mono `L + R` source
//!   reads as the stereo sum of a mono signal did).
//! - A LOUDNESS `Mode` change resets the meter: the five histories, their maxima, integrated loudness and range, and the
//!   momentary and short-term windows. The spectrum, scope, vectorscope and pitch keep theirs.
//! - The spectrum is a Hann-windowed 2048-point FFT every 1024 samples, its magnitude normalised so a full-scale sine
//!   reads 0 dB.
//! - The oscilloscope keeps the last second as the min and max of each quarter millisecond.
//! - The loudness meters (one entry each 50 ms): Peak and RMS over the entry; True Peak 4× oversampled (Catmull-Rom);
//!   LUFS momentary (0.4 s) and short-term (3 s) of BS.1770's K-weighted channel sum. Integrated loudness gates the
//!   momentary values (−70 LUFS, then −10 LU) and the loudness range the short-term ones (−70, then −20 LU; 10th to 95th
//!   percentile), each through a histogram of 0.1 LU bins from −70 to +10 LUFS, so they run in fixed memory.
//! - The pitch track takes the Tuner's detector's last reading for each 50 ms entry.

use super::{Correlation, Detector, PETAL_BINS, Petal};
use crate::biquad::flush;

/// FFT size and its bins.
pub const FFT: usize = 2048;
pub const BINS: usize = FFT / 2 + 1;
/// Oscilloscope buckets: one per quarter millisecond over 1 s (finer than the default 0.1 s span's 2 px columns).
pub const SCOPE: usize = 4000;
/// Buckets per millisecond.
pub const SCOPE_PER_MS: usize = SCOPE / 1000;
/// Loudness and pitch history entries: 400 and 200 of 50 ms (20 s and 10 s).
pub const LOUDNESS: usize = 400;
pub const PITCH: usize = 200;
/// The loudness meters, in the `Mode` spinner's order reversed to the meter law: Peak, True Peak, RMS, LUFS 0.4 s,
/// LUFS 3 s.
pub const METERS: usize = 5;
/// Interferometer correlations kept: 2 s of 2048-sample blocks at up to 192 kHz.
pub const CORRELATIONS: usize = 188;
/// The floor of every dB value: silence, or no data.
pub const FLOOR: f32 = -120.0;

/// The slab's layout: offsets of each part.
pub const SPECTRUM_AT: usize = 0;
pub const SCOPE_AT: usize = SPECTRUM_AT + BINS;
pub const LOUDNESS_AT: usize = SCOPE_AT + 1 + 2 * SCOPE;
/// After the loudness histories: the 5 running maxima, then integrated loudness and loudness range.
pub const SUMMARY_AT: usize = LOUDNESS_AT + 1 + METERS * LOUDNESS;
pub const PETAL_AT: usize = SUMMARY_AT + METERS + 2;
pub const CORRELATION_AT: usize = PETAL_AT + 1 + PETAL_BINS;
pub const PITCH_AT: usize = CORRELATION_AT + 1 + CORRELATIONS;
pub const LEN: usize = PITCH_AT + 1 + PITCH;

/// The panel's settings the analysis takes, as the engine's `SetSetting` brings them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    /// `Channel`, the source of every view but the vectorscope: 0 Left, 1 Right, 2 L + R ((L + R)/2).
    Channel(usize),
    /// `Reactivity`, the falling smoothing: 0 Slow, 1 Medium, 2 Fast.
    Reactivity(usize),
    /// `DC Block` on.
    DcBlock(bool),
    /// `Input Gain` as a factor, ×0.1…×10.
    InputGain(f32),
    /// LOUDNESS `Mode`, its item: choosing another resets the meter.
    LoudnessMode(usize),
}

/// Histogram bins of 0.1 LU from −70 to +10 LUFS.
const HISTOGRAM: usize = 800;

/// A biquad in direct form I, f64.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn run(&mut self, input: f64) -> f64 {
        let y = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [if y.abs() < 1e-30 { 0.0 } else { y }, self.y[0]];
        self.y[0]
    }
}

/// BS.1770's K-weighting at `rate`: the high shelf, then the high pass (the standard coefficients at 48 kHz).
fn k_weighting(rate: f64) -> [Biquad; 2] {
    let (f0, gain, q) = (1681.974450955533, 3.999843853973347, 0.7071752369554196);
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let vh = 10f64.powf(gain / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        ..Biquad::default()
    };
    let (f0, q) = (38.13547087602444, 0.5003270373238773);
    let k = (std::f64::consts::PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        ..Biquad::default()
    };
    [shelf, high_pass]
}

/// A loudness histogram: per 0.1 LU bin from −70 LUFS, the count and the summed mean squares.
struct Histogram {
    count: Vec<u32>,
    energy: Vec<f64>,
}

impl Histogram {
    fn new() -> Self {
        Self {
            count: vec![0; HISTOGRAM],
            energy: vec![0.0; HISTOGRAM],
        }
    }

    fn reset(&mut self) {
        self.count.fill(0);
        self.energy.fill(0.0);
    }

    /// Adds a block of mean square `ms` above the absolute gate (−70 LUFS).
    fn add(&mut self, ms: f64) {
        let lufs = lufs(ms);
        if lufs > -70.0 {
            let bin = (((lufs + 70.0) * 10.0) as usize).min(HISTOGRAM - 1);
            self.count[bin] += 1;
            self.energy[bin] += ms;
        }
    }

    /// The first bin at or above `lufs`.
    fn bin(lufs: f64) -> usize {
        (((lufs + 70.0) * 10.0).ceil().max(0.0) as usize).min(HISTOGRAM)
    }

    /// The mean square over the bins from `from` on, and their count.
    fn mean(&self, from: usize) -> Option<f64> {
        let n: u64 = self.count[from..].iter().map(|&c| u64::from(c)).sum();
        (n > 0).then(|| self.energy[from..].iter().sum::<f64>() / n as f64)
    }

    /// Integrated loudness: the mean of the blocks above the relative gate, 10 LU under the gated mean.
    fn integrated(&self) -> Option<f64> {
        let gate = lufs(self.mean(0)?) - 10.0;
        self.mean(Self::bin(gate)).map(lufs)
    }

    /// Loudness range: the 10th to 95th percentile of the blocks above 20 LU under the gated mean.
    fn range(&self) -> Option<f64> {
        let from = Self::bin(lufs(self.mean(0)?) - 20.0);
        let n: u64 = self.count[from..].iter().map(|&c| u64::from(c)).sum();
        if n == 0 {
            return None;
        }
        let percentile = |p: f64| {
            let want = (p * n as f64).ceil().max(1.0) as u64;
            let mut seen = 0;
            for (i, &c) in self.count[from..].iter().enumerate() {
                seen += u64::from(c);
                if seen >= want {
                    return (from + i) as f64 / 10.0 - 70.0;
                }
            }
            (HISTOGRAM - 1) as f64 / 10.0 - 70.0
        };
        Some(percentile(0.95) - percentile(0.1))
    }
}

/// LUFS of a K-weighted channel-sum mean square.
fn lufs(ms: f64) -> f64 {
    if ms > 0.0 {
        -0.691 + 10.0 * ms.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// dB of a level, at the floor for nothing.
fn db(level: f64) -> f32 {
    if level > 0.0 {
        (20.0 * level.log10()).max(f64::from(FLOOR)) as f32
    } else {
        FLOOR
    }
}

pub(super) struct Analyzer {
    // Settings: the source (0 Left, 1 Right, 2 L + R), the spectrum's falling weight, the input gain, the DC blocker
    // (its pole, and per channel its last input and output, while on) and the loudness mode.
    source: usize,
    falling: f32,
    gain: f32,
    dc_pole: f32,
    dc: Option<[[f32; 2]; 2]>,
    mode: usize,
    // Spectrum.
    window: Vec<f32>,
    ring: Vec<f32>,
    /// The ring slot the next sample goes to, and whether the ring has been round once.
    written: usize,
    filled: bool,
    since_fft: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    reversed: Vec<u16>,
    norm: f32,
    // Oscilloscope.
    bucket: usize,
    in_bucket: usize,
    low: f32,
    high: f32,
    // Loudness.
    entry: usize,
    in_entry: usize,
    peak: f64,
    true_peak: f64,
    squares: f64,
    k: [Biquad; 2],
    k_energy: f64,
    /// The K-weighted energy of each of the last 60 entries (3 s), for the momentary and short-term windows.
    energies: Vec<f64>,
    energy_at: usize,
    last: [f64; 3],
    momentary: Histogram,
    short_term: Histogram,
    // Vectorscope and interferometer.
    petal: Petal,
    correlation: Correlation,
    // Pitch.
    detector: Detector,
    pitch: f32,
}

impl Analyzer {
    pub(super) fn new(sample_rate: f32) -> Self {
        let n = FFT as f32;
        let window: Vec<f32> = (0..FFT)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n).cos())
            .collect();
        let norm = 2.0 / window.iter().sum::<f32>();
        let bits = FFT.trailing_zeros();
        Self {
            source: 2,
            falling: 0.95,
            gain: 1.0,
            dc_pole: (-std::f32::consts::TAU * 10.0 / sample_rate).exp(),
            dc: None,
            mode: 0,
            window,
            ring: vec![0.0; FFT],
            written: 0,
            filled: false,
            since_fft: 0,
            re: vec![0.0; FFT],
            im: vec![0.0; FFT],
            cos: (0..FFT / 2)
                .map(|i| (std::f32::consts::TAU * i as f32 / n).cos())
                .collect(),
            sin: (0..FFT / 2)
                .map(|i| (std::f32::consts::TAU * i as f32 / n).sin())
                .collect(),
            reversed: (0..FFT)
                .map(|i| ((i as u32).reverse_bits() >> (32 - bits)) as u16)
                .collect(),
            norm,
            bucket: ((sample_rate / SCOPE as f32).round() as usize).max(1),
            in_bucket: 0,
            low: f32::INFINITY,
            high: f32::NEG_INFINITY,
            entry: ((sample_rate * 0.05).round() as usize).max(1),
            in_entry: 0,
            peak: 0.0,
            true_peak: 0.0,
            squares: 0.0,
            k: k_weighting(f64::from(sample_rate)),
            k_energy: 0.0,
            energies: vec![0.0; 60],
            energy_at: 0,
            last: [0.0; 3],
            momentary: Histogram::new(),
            short_term: Histogram::new(),
            petal: Petal::new(0.9),
            correlation: Correlation::default(),
            detector: Detector::new(sample_rate),
            pitch: 0.0,
        }
    }

    /// Back to rest: nothing heard. The settings stay.
    pub(super) fn reset(&mut self, slab: &mut [f32]) {
        self.ring.fill(0.0);
        (self.written, self.filled, self.since_fft) = (0, false, 0);
        (self.in_bucket, self.in_entry) = (0, 0);
        (self.low, self.high) = (f32::INFINITY, f32::NEG_INFINITY);
        (self.peak, self.true_peak, self.squares, self.k_energy) = (0.0, 0.0, 0.0, 0.0);
        self.k = self.k.map(|b| Biquad {
            x: [0.0; 2],
            y: [0.0; 2],
            ..b
        });
        self.dc = self.dc.map(|_| [[0.0; 2]; 2]);
        self.last = [0.0; 3];
        self.petal.reset();
        self.correlation = Correlation::default();
        self.detector.reset();
        self.pitch = 0.0;
        slab.fill(0.0);
        slab[SPECTRUM_AT..SPECTRUM_AT + BINS].fill(FLOOR);
        self.reset_loudness(slab);
    }

    /// The loudness meter back to rest: its histories, maxima, summary and windows.
    fn reset_loudness(&mut self, slab: &mut [f32]) {
        self.energies.fill(0.0);
        self.energy_at = 0;
        self.momentary.reset();
        self.short_term.reset();
        slab[LOUDNESS_AT + 1..SUMMARY_AT + METERS].fill(FLOOR);
        slab[SUMMARY_AT + METERS..PETAL_AT].fill(f32::NAN);
    }

    /// Takes `setting`; an index past the last item is the last.
    pub(super) fn set(&mut self, slab: &mut [f32], setting: Setting) {
        match setting {
            Setting::Channel(source) => self.source = source.min(2),
            Setting::Reactivity(item) => {
                let item = item.min(2);
                self.falling = [0.999, 0.95, 0.7][item];
                self.petal.falling = [0.99, 0.9, 0.5][item];
            }
            Setting::DcBlock(on) => {
                if on != self.dc.is_some() {
                    self.dc = on.then_some([[0.0; 2]; 2]);
                }
            }
            Setting::InputGain(gain) => {
                if gain.is_finite() && gain > 0.0 {
                    self.gain = gain;
                }
            }
            Setting::LoudnessMode(mode) => {
                if mode.min(4) != self.mode {
                    self.mode = mode.min(4);
                    self.reset_loudness(slab);
                }
            }
        }
    }

    /// Channel `channel`'s input after the DC blocker (while on) and the input gain.
    fn condition(&mut self, channel: usize, x: f32) -> f32 {
        let x = match &mut self.dc {
            Some(dc) => {
                let [last_in, last_out] = &mut dc[channel];
                let y = flush(x - *last_in + self.dc_pole * *last_out);
                (*last_in, *last_out) = (x, y);
                y
            }
            None => x,
        };
        x * self.gain
    }

    /// One input frame.
    pub(super) fn frame(&mut self, slab: &mut [f32], (l, r): (f32, f32)) {
        let (l, r) = (self.condition(0, l), self.condition(1, r));
        let mono = match self.source {
            0 => l,
            1 => r,
            _ => 0.5 * (l + r),
        };
        // Spectrum.
        self.ring[self.written] = mono;
        self.written = (self.written + 1) % FFT;
        self.filled |= self.written == 0;
        self.since_fft += 1;
        if self.since_fft >= FFT / 2 && self.filled {
            self.since_fft = 0;
            self.spectrum(&mut slab[SPECTRUM_AT..SPECTRUM_AT + BINS]);
        }
        // Oscilloscope.
        self.low = self.low.min(mono);
        self.high = self.high.max(mono);
        self.in_bucket += 1;
        if self.in_bucket == self.bucket {
            let next = slab[SCOPE_AT] as usize % SCOPE;
            slab[SCOPE_AT + 1 + 2 * next] = self.low;
            slab[SCOPE_AT + 2 + 2 * next] = self.high;
            slab[SCOPE_AT] = ((next + 1) % SCOPE) as f32;
            (self.in_bucket, self.low, self.high) = (0, f32::INFINITY, f32::NEG_INFINITY);
        }
        // Loudness.
        let x = f64::from(mono);
        let [a, b, c] = self.last;
        self.true_peak = self.true_peak.max(x.abs());
        for t in [0.25, 0.5, 0.75] {
            // Catmull-Rom between b and c, with a and x around.
            let v = 0.5
                * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - x) * t * t
                    + (3.0 * b - a - 3.0 * c + x) * t * t * t);
            self.true_peak = self.true_peak.max(v.abs());
        }
        self.last = [b, c, x];
        self.peak = self.peak.max(x.abs());
        self.squares += x * x;
        // The source as both channels of the sum.
        let [shelf, high_pass] = &mut self.k;
        let weighted = high_pass.run(shelf.run(x));
        self.k_energy += 2.0 * weighted * weighted;
        // Vectorscope, interferometer, pitch.
        self.petal
            .frame(&mut slab[PETAL_AT..CORRELATION_AT], (l, r));
        if let Some(c) = self.correlation.frame((l, r)) {
            let next = slab[CORRELATION_AT] as usize % CORRELATIONS;
            slab[CORRELATION_AT + 1 + next] = c;
            slab[CORRELATION_AT] = ((next + 1) % CORRELATIONS) as f32;
        }
        if let Some(hz) = self.detector.frame(mono) {
            self.pitch = hz;
        }
        self.in_entry += 1;
        if self.in_entry == self.entry {
            self.entry_done(slab);
        }
    }

    /// A 50 ms entry of the loudness and pitch histories.
    fn entry_done(&mut self, slab: &mut [f32]) {
        let n = self.entry as f64;
        self.energies[self.energy_at] = self.k_energy / n;
        self.energy_at = (self.energy_at + 1) % self.energies.len();
        let window = |entries: usize| {
            let len = self.energies.len();
            (1..=entries)
                .map(|back| self.energies[(self.energy_at + len - back) % len])
                .sum::<f64>()
                / entries as f64
        };
        let (momentary, short_term) = (window(8), window(60));
        self.momentary.add(momentary);
        self.short_term.add(short_term);
        let values = [
            db(self.peak),
            db(self.true_peak),
            db((self.squares / n).sqrt()),
            lufs(momentary).max(f64::from(FLOOR)) as f32,
            lufs(short_term).max(f64::from(FLOOR)) as f32,
        ];
        let next = slab[LOUDNESS_AT] as usize % LOUDNESS;
        for (meter, value) in values.into_iter().enumerate() {
            slab[LOUDNESS_AT + 1 + meter * LOUDNESS + next] = value;
            let max = &mut slab[SUMMARY_AT + meter];
            *max = max.max(value);
        }
        slab[LOUDNESS_AT] = ((next + 1) % LOUDNESS) as f32;
        let summary = [self.momentary.integrated(), self.short_term.range()];
        for (i, value) in summary.into_iter().enumerate() {
            slab[SUMMARY_AT + METERS + i] = value.map_or(f32::NAN, |v| v as f32);
        }
        let next = slab[PITCH_AT] as usize % PITCH;
        slab[PITCH_AT + 1 + next] = self.pitch;
        slab[PITCH_AT] = ((next + 1) % PITCH) as f32;
        (
            self.in_entry,
            self.peak,
            self.true_peak,
            self.squares,
            self.k_energy,
        ) = (0, 0.0, 0.0, 0.0, 0.0);
    }

    /// The FFT of the last [`FFT`] samples into the smoothed spectrum `out` (dB): rising with old-value weight 0.7,
    /// falling with `Reactivity`'s.
    fn spectrum(&mut self, out: &mut [f32]) {
        for i in 0..FFT {
            let j = usize::from(self.reversed[i]);
            self.re[j] = self.ring[(self.written + i) % FFT] * self.window[i];
            self.im[j] = 0.0;
        }
        let mut size = 2;
        while size <= FFT {
            let (half, step) = (size / 2, FFT / size);
            for start in (0..FFT).step_by(size) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step], -self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let (re, im) = (
                        self.re[b] * c - self.im[b] * s,
                        self.re[b] * s + self.im[b] * c,
                    );
                    (self.re[b], self.im[b]) = (self.re[a] - re, self.im[a] - im);
                    (self.re[a], self.im[a]) = (self.re[a] + re, self.im[a] + im);
                }
            }
            size *= 2;
        }
        for (bin, shown) in out.iter_mut().enumerate() {
            let magnitude = (self.re[bin].hypot(self.im[bin]) * self.norm).max(1e-6);
            let new = (20.0 * magnitude.log10()).max(FLOOR);
            let weight = if new > *shown { 0.7 } else { self.falling };
            *shown = flush(weight * *shown + (1.0 - weight) * new);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_counters_stay_inside_their_rings() {
        // A running count would overflow wasm32's usize after about a day of audio at 48 kHz.
        let mut analyzer = Analyzer::new(48_000.0);
        let mut slab = vec![0.0; LEN];
        analyzer.reset(&mut slab);
        for _ in 0..3 * FFT + 7 {
            analyzer.frame(&mut slab, (0.1, 0.1));
            assert!(analyzer.written < FFT, "{}", analyzer.written);
            let detector = &analyzer.detector;
            assert!(
                detector.written < detector.ring.len(),
                "{}",
                detector.written
            );
        }
    }
}
