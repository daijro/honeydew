//! Slicer (ID 33): one loop cut into slices at its transients; slice k plays on key first + k.
//!
//! [`detect`] slices off the audio thread (the app thread, or a loop pick's worker) whenever the loop or the
//! Sensitivity changes, and the [`Slicer`] gets the slice starts as an `Arc`: the audio thread only plays them.
//! [`auto_clip`] is AUTO's pure half, the notes a re-detect writes into the current clip.

use std::f64::consts::TAU;
use std::ops::Range;
use std::sync::Arc;

use crate::ProcessContext;
use crate::instrument::Instrument;
use crate::sample::SampleData;

/// The Sensitivity before any `CSTM` is read (detector step 9). A new module writes 0.25 (the panel's default).
pub const SENSITIVITY_BEFORE_CSTM: f32 = 0.33;
/// At most this many slices.
pub const MAX_SLICES: usize = 128;
/// Slice 0's key when no other Slicer on the channel plays a key.
pub const DEFAULT_FIRST_NOTE: u8 = 48;
/// The velocity of AUTO's notes.
pub const AUTO_VELOCITY: f32 = 0.7;

// Detector D. Every constant is in file frames, as authored at 44.1 kHz: at other rates the times
// they stand for scale by 44100/rate.
const HOP: usize = 16;
const WINDOW: usize = 128;
const HIGH_PASS_A: f64 = 9.134e-5;
const HIGH_PASS_B: f64 = 0.96;
const HOPS_PER_MS: f64 = 2.7562;
/// sens·D ≥ 1 is an onset; its start moves back to the earliest frame of the last 28 with sens·D > 0.75.
const ONSET: f64 = 1.0;
const START: f64 = 0.75;
const BACK_HOPS: usize = 28;
/// Then forward to the first frame whose RMS (normalised) is above 5 %, then back to a zero crossing within 20 ms.
const RMS_FLOOR: f64 = 0.05;
const ZERO_CROSSING_FRAMES: usize = 882;
/// The next search resumes at frame round((pos + 2646 + 64)/16): slices are at least about 60 ms long.
const MIN_SLICE: usize = 2646;

/// One detector band (detector steps 4–5).
struct Band {
    /// The FFT bins it averages.
    bins: Range<usize>,
    /// Max-hold over frames i − hold … i.
    hold: usize,
    /// The envelope's release time constant.
    ms: f64,
    k: f64,
    floor: f64,
}

const LOW: Band = Band {
    bins: 1..6,
    hold: 55,
    ms: 15.0,
    k: 4.5,
    floor: 1e-10 / 1.8,
};
const HIGH: Band = Band {
    bins: 6..64,
    hold: 41,
    ms: 10.0,
    k: 5.0,
    floor: 1e-10 / 20.0,
};

/// The slice start frames of `sample` at `sensitivity`, ascending, at most [`MAX_SLICES`]. Sensitivity > 0 detects
/// onsets on the first channel (detector D). Sensitivity 0 (`M`) takes the file's cue markers. Either way, a
/// loop without any is one whole-file slice; an empty sample has none. App thread: it allocates.
pub fn detect(sample: &SampleData, sensitivity: f32) -> Vec<usize> {
    let frames = sample.frames();
    let mut starts = if sensitivity > 0.0 {
        onsets(sample, f64::from(sensitivity))
    } else {
        let mut cues: Vec<usize> = sample.meta.cues.iter().map(|&cue| cue as usize).collect();
        cues.retain(|&cue| cue < frames);
        cues.sort_unstable();
        cues.dedup();
        cues.truncate(MAX_SLICES);
        cues
    };
    // No onset at all (a silent loop, or one too smooth for the Sensitivity) is one whole slice.
    if starts.is_empty() && frames > 0 {
        starts.push(0);
    }
    starts
}

fn onsets(sample: &SampleData, sensitivity: f64) -> Vec<usize> {
    // 1. A copy of the first channel, normalised to peak 1 over all channels.
    let channels = sample
        .left
        .iter()
        .chain(sample.right.iter().flat_map(|r| r.iter()));
    let peak = channels.fold(0.0_f32, |peak, s| peak.max(s.abs()));
    if peak == 0.0 {
        return Vec::new();
    }
    // The normalised first channel, read in place; the high-passes work on a copy.
    let x = |k: usize| f64::from(sample.left[k]) / f64::from(peak);
    let mut y: Vec<f64> = (0..sample.frames()).map(x).collect();
    // 2. Three high-passes.
    for _ in 0..3 {
        high_pass(&mut y);
    }
    let d = detection(&y);
    // 7–8. Onsets, their starts, and the next search after each.
    let rms = |frame: usize| {
        let window = frame * HOP..(frame * HOP + WINDOW).min(y.len());
        (window.map(|k| x(k) * x(k)).sum::<f64>() / WINDOW as f64).sqrt()
    };
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < d.len() && starts.len() < MAX_SLICES {
        if sensitivity * d[i] < ONSET {
            i += 1;
            continue;
        }
        let back = (i.saturating_sub(BACK_HOPS)..=i)
            .find(|&j| sensitivity * d[j] > START)
            .unwrap_or(i);
        // The forward move stops at the onset's own frame.
        let frame = (back..=i).find(|&j| rms(j) > RMS_FLOOR).unwrap_or(i);
        let pos = zero_crossing(x, frame * HOP);
        // Starts only grow: the search resumes at least 2703 frames on, and a start moves back at most 28 hops plus
        // 882 frames. "minimum slice 60 ms" holds for the last slice too, so no slice starts in the
        // last 60 ms (a loop cut mid-sound splatters into the zero-padded tail frames).
        if y.len() - pos >= MIN_SLICE {
            starts.push(pos);
        }
        i = ((pos + MIN_SLICE + WINDOW / 2 + HOP / 2) / HOP).max(i + 1);
    }
    starts
}

/// One high-pass from zero state: u = v + a(x − p); p += u; v = b·u; y = x − p.
fn high_pass(x: &mut [f64]) {
    let (mut p, mut v) = (0.0, 0.0);
    for s in x {
        let u = v + HIGH_PASS_A * (*s - p);
        p += u;
        v = HIGH_PASS_B * u;
        *s -= p;
    }
}

/// D per analysis frame (detector steps 3–6): frames of 128 at hop 16, zero-padded, n = floor((N + 143)/16).
fn detection(y: &[f64]) -> Vec<f64> {
    let n = (y.len() + WINDOW).div_ceil(HOP);
    let window: Vec<f64> = (0..WINDOW)
        .map(|k| 0.53836 - 0.46164 * (TAU * k as f64 / WINDOW as f64).cos())
        .collect();
    let gain: f64 = window.iter().sum();
    let (mut low, mut high) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut re, mut im) = ([0.0; WINDOW], [0.0; WINDOW]);
    for i in 0..n {
        for (k, (re, im)) in re.iter_mut().zip(&mut im).enumerate() {
            *re = y.get(i * HOP + k).map_or(0.0, |s| s * window[k]);
            *im = 0.0;
        }
        fft(&mut re, &mut im);
        let mean = |bins: Range<usize>| {
            let count = bins.len() as f64;
            bins.map(|k| (re[k].hypot(im[k]) / gain).powi(2))
                .sum::<f64>()
                / count
        };
        low.push(mean(LOW.bins));
        high.push(mean(HIGH.bins));
    }
    let (low, high) = (LOW.detect(&low), HIGH.detect(&high));
    low.iter().zip(&high).map(|(l, h)| l.max(*h)).collect()
}

impl Band {
    /// D_b per frame from the raw band: max-hold, then an envelope that follows rises at once and falls with time
    /// constant T; D_b = k·(E[i]/r − 1) where E[i] ≥ r = max(E[i − 1], ε), else 0.
    fn detect(&self, raw: &[f64]) -> Vec<f64> {
        let fall = (-1.0 / (HOPS_PER_MS * self.ms)).exp();
        let mut envelope = 0.0_f64;
        (0..raw.len())
            .map(|i| {
                let held = raw[i.saturating_sub(self.hold)..=i]
                    .iter()
                    .fold(0.0_f64, |held, &v| held.max(v));
                let r = envelope.max(self.floor);
                envelope = if held > envelope {
                    held
                } else {
                    held + (envelope - held) * fall
                };
                if envelope >= r {
                    self.k * (envelope / r - 1.0)
                } else {
                    0.0
                }
            })
            .collect()
    }
}

/// `pos`, moved back to the latest zero crossing of `x` within the 20 ms before it (a frame that is 0, or whose sign
/// differs from the one before); `pos` itself when there is none.
fn zero_crossing(x: impl Fn(usize) -> f64, pos: usize) -> usize {
    (pos.saturating_sub(ZERO_CROSSING_FRAMES).max(1)..=pos)
        .rev()
        .find(|&k| x(k) == 0.0 || (x(k - 1) < 0.0) != (x(k) < 0.0))
        .unwrap_or(pos)
}

/// In-place radix-2 FFT (decimation in time) of `re` + i·`im`, whose equal length is a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let (sin, cos) = (-TAU / len as f64).sin_cos();
        for start in (0..n).step_by(len) {
            let (mut wr, mut wi) = (1.0, 0.0);
            for k in start..start + len / 2 {
                let m = k + len / 2;
                let (tr, ti) = (re[m] * wr - im[m] * wi, re[m] * wi + im[m] * wr);
                (re[m], im[m]) = (re[k] - tr, im[k] - ti);
                re[k] += tr;
                im[k] += ti;
                (wr, wi) = (wr * cos - wi * sin, wr * sin + wi * cos);
            }
        }
        len <<= 1;
    }
}

/// Slice 0's key: one above `highest`, the highest key another Slicer on the channel plays, else 48;
/// lowered so that the last of `slices` keys is at most 127.
pub fn first_note(highest: Option<u8>, slices: usize) -> u8 {
    let first = highest.map_or(usize::from(DEFAULT_FIRST_NOTE), |key| usize::from(key) + 1);
    first.min(128 - slices.clamp(1, 128)) as u8
}

/// AUTO's clip length in beats for a loop of `frames`: frames × tempo/(60·44100) when the file
/// carries its tempo, else max(4, the next power of 2 ≥ trunc(0.75·b)), b = frames × song BPM/(44100·60). Both
/// assume 44.1 kHz whatever the file's rate.
pub fn auto_clip_beats(frames: usize, loop_tempo: Option<f32>, song_bpm: f64) -> f64 {
    const FRAMES_PER_MINUTE: f64 = 60.0 * 44_100.0;
    match loop_tempo {
        Some(tempo) => frames as f64 * f64::from(tempo) / FRAMES_PER_MINUTE,
        None => {
            let b = frames as f64 * song_bpm / FRAMES_PER_MINUTE;
            ((0.75 * b) as u64).next_power_of_two().max(4) as f64
        }
    }
}

/// One note AUTO writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoNote {
    pub key: u8,
    pub start_beats: f64,
    pub length_beats: f64,
    pub velocity: f32,
}

/// AUTO's notes: one per slice of a loop of `frames`, on key `first` + k, velocity 0.7.
/// The loop is laid over the clip's `clip_beats`, so a slice starts at start/frames × clip_beats
/// and lasts until the next slice (the last until the clip end); playback stays at the loop's own speed.
pub fn auto_clip(slices: &[usize], frames: usize, first: u8, clip_beats: f64) -> Vec<AutoNote> {
    let beat = |frame: usize| frame as f64 / frames as f64 * clip_beats;
    (slices.iter().enumerate())
        .filter_map(|(k, &start)| {
            let key = u8::try_from(usize::from(first) + k)
                .ok()
                .filter(|&key| key <= 127)?;
            let end = slices.get(k + 1).copied().unwrap_or(frames);
            Some(AutoNote {
                key,
                start_beats: beat(start),
                length_beats: beat(end) - beat(start),
                velocity: AUTO_VELOCITY,
            })
        })
        .collect()
}

/// Voices in the pool, all allocated at build.
// Polyphony uses a fixed pool; beyond it a new slice takes the oldest voice.
// ponytail: fixed pool of 256; grow it if a song ever stacks more slices at once.
const VOICES: usize = 256;
/// The fast fade: target 0 with c = 0.01 per sample (the pad choke's), for MONO and Hold's release.
// Hold's "fades at 0.01 per sample" is this same exponential fast fade, not a linear ramp.
const FAST_FADE: f64 = 0.01;
/// The linear fade before a slice's end, in source frames.
const END_FADE: f64 = 512.0;
/// A fading or decaying voice ends below this.
const SILENT: f64 = 1e-4;
/// The params: Enable, Pitch, hidden p2, Attack, Decay, Sustain, MONO; their defaults.
const DEFAULTS: [f32; 7] = [1.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0];
const PITCH: usize = 1;
const ATTACK: usize = 3;
const DECAY: usize = 4;
const SUSTAIN: usize = 5;
const MONO: usize = 6;

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    /// At 1, decaying toward 0 unless Decay > 0.99.
    Decay,
    Fade,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    stage: Stage,
    key: u8,
    /// Sustain Hold: a note-off fades the voice.
    hold: bool,
    /// The source frame being read.
    pos: f64,
    /// The slice's end, where the 512-frame fade dips to 0.
    slice_end: f64,
    /// Where the voice ends: the slice end (Off) or the file end (Hold, To End).
    end: f64,
    gains: (f32, f32),
    env: f64,
    /// The attack coefficient, fixed at the start.
    attack: f64,
    age: u64,
}

/// The Slicer instrument: plays the slices of one loop. Built on the app thread with its loop, slice starts and first
/// key; nothing here allocates afterwards.
pub struct Slicer {
    sample: Option<Arc<SampleData>>,
    slices: Arc<[usize]>,
    first: u8,
    device_rate: f32,
    params: [f32; 7],
    voices: Vec<Voice>,
    age: u64,
}

impl Slicer {
    /// The Slicer at `device_rate` playing `sample` (`None`: no loop, or a missing one, so silent) cut at `slices`
    /// ([`detect`]), slice 0 on key `first`.
    pub fn new(
        device_rate: f32,
        sample: Option<Arc<SampleData>>,
        slices: Arc<[usize]>,
        first: u8,
    ) -> Self {
        Self {
            sample,
            slices,
            first,
            device_rate,
            params: DEFAULTS,
            voices: vec![Voice::default(); VOICES],
            age: 0,
        }
    }

    /// The pad envelope's x = 0.95v + 0.05 as x⁴.
    fn x4(&self, index: usize) -> f64 {
        (0.95 * f64::from(self.params[index]) + 0.05).powi(4)
    }
}

/// Pitch bend and slide events are ignored: the trait's no-op `set_controller`.
impl Instrument for Slicer {
    fn set_parameter(&mut self, index: usize, value: f32) {
        if let Some(param) = self.params.get_mut(index) {
            *param = value;
        }
    }

    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        let Some(sample) = &self.sample else { return };
        let Some(k) = key.checked_sub(self.first).map(usize::from) else {
            return;
        };
        // Keys past the last slice play nothing.
        let Some(&start) = self.slices.get(k) else {
            return;
        };
        let frames = sample.frames();
        let slice_end = self.slices.get(k + 1).copied().unwrap_or(frames);
        // MONO is on at any nonzero value.
        if self.params[MONO] != 0.0 {
            for voice in self.voices.iter_mut().filter(|v| v.stage != Stage::Idle) {
                voice.stage = Stage::Fade;
            }
        }
        // Off 0, Hold 1, To End 2: trunc(2v) in f64.
        let sustain = (f64::from(self.params[SUSTAIN]) * 2.0).trunc();
        let attack = if self.params[ATTACK] < 0.001 {
            1.0
        } else {
            0.0002 / (500.0 * self.x4(ATTACK))
        };
        let (velocity, pan) = (velocity.clamp(0.0, 1.0), pan.clamp(0.0, 1.0));
        self.age += 1;
        let voice = (self.voices.iter_mut())
            .min_by_key(|v| (v.stage != Stage::Idle, v.age))
            .expect("VOICES > 0");
        *voice = Voice {
            stage: Stage::Attack,
            key,
            hold: sustain == 1.0,
            pos: start as f64,
            slice_end: slice_end as f64,
            end: if sustain == 0.0 { slice_end } else { frames } as f64,
            gains: (
                velocity * (2.0 - 2.0 * pan).min(1.0),
                velocity * (2.0 * pan).min(1.0),
            ),
            env: 0.0,
            attack,
            age: self.age,
        };
    }

    /// Sustain Hold fades the key's voices; Off and To End play on.
    fn note_off(&mut self, key: u8) {
        for voice in self.voices.iter_mut() {
            if voice.stage != Stage::Idle && voice.key == key && voice.hold {
                voice.stage = Stage::Fade;
            }
        }
    }

    fn all_notes_off(&mut self) {
        for voice in self.voices.iter_mut() {
            if voice.stage != Stage::Idle && voice.hold {
                voice.stage = Stage::Fade;
            }
        }
    }

    fn reset(&mut self) {
        self.voices.fill(Voice::default());
    }

    /// Enable off freezes the voices; they resume when it is back on.
    fn freezes_when_disabled(&self) -> bool {
        true
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], _context: ProcessContext) {
        let Some(sample) = &self.sample else { return };
        // Pitch and speed together, by resampling: ratio 2^(s·(1/12)), all in f64.
        let semitones = pitch_steps(self.params[PITCH]);
        let step = f64::from(sample.sample_rate) / f64::from(self.device_rate)
            * (semitones * (1.0 / 12.0)).exp2();
        // The pad envelope's coefficients: instant (c = 1) when v < 0.001 or 50x⁴ < 0.001; Decay > 0.99 = no decay.
        let decay = match (self.params[DECAY], 50.0 * self.x4(DECAY)) {
            (v, _) if v > 0.99 => 0.0,
            (v, slow) if v < 0.001 || slow < 0.001 => 1.0,
            (_, slow) => 0.0002 / slow,
        };
        let (source_l, source_r) = (
            &sample.left[..],
            sample.right.as_deref().unwrap_or(&sample.left),
        );
        for voice in self.voices.iter_mut().filter(|v| v.stage != Stage::Idle) {
            for (out_l, out_r) in left.iter_mut().zip(right.iter_mut()) {
                match voice.stage {
                    Stage::Attack => {
                        voice.env += (5.0 - voice.env) * voice.attack;
                        if voice.env > 1.0 {
                            voice.env = 1.0;
                            voice.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => voice.env -= voice.env * decay,
                    Stage::Fade => voice.env -= voice.env * FAST_FADE,
                    Stage::Idle => {}
                }
                if voice.pos >= voice.end || (voice.stage != Stage::Attack && voice.env < SILENT) {
                    voice.stage = Stage::Idle;
                    break;
                }
                let dip = ((voice.slice_end - voice.pos) / END_FADE).clamp(0.0, 1.0);
                let dip = if voice.pos < voice.slice_end {
                    dip
                } else {
                    1.0
                };
                let gain = (voice.env * dip) as f32;
                *out_l += lerp(source_l, voice.pos) * gain * voice.gains.0;
                *out_r += lerp(source_r, voice.pos) * gain * voice.gains.1;
                voice.pos += step;
            }
        }
    }
}

/// Pitch (p1) in semitones: s = trunc(f64(v)·48 − 24), exact, so at v = f32(k/48)
/// some k land one semitone off k − 24. The playback ratio and the `%d` text both use it.
pub fn pitch_steps(v: f32) -> f64 {
    (f64::from(v) * 48.0 - 24.0).trunc()
}

/// `x` at fractional frame `pos` (< its length) by linear interpolation; the last frame holds.
fn lerp(x: &[f32], pos: f64) -> f32 {
    let i = pos as usize;
    let (a, b) = (x[i], x.get(i + 1).copied().unwrap_or(x[i]));
    a + (b - a) * (pos - i as f64) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fft_matches_a_dft() {
        let input: Vec<f64> = (0..128)
            .map(|k| ((k * 37 % 101) as f64 / 50.0 - 1.0) + (TAU * 5.3 * k as f64 / 128.0).sin())
            .collect();
        let (mut re, mut im) = (input.clone(), vec![0.0; 128]);
        fft(&mut re, &mut im);
        for bin in 0..128 {
            let (mut dr, mut di) = (0.0, 0.0);
            for (k, x) in input.iter().enumerate() {
                let phase = -TAU * (bin * k) as f64 / 128.0;
                dr += x * phase.cos();
                di += x * phase.sin();
            }
            assert!(
                (re[bin] - dr).abs() < 1e-9 && (im[bin] - di).abs() < 1e-9,
                "bin {bin}"
            );
        }
    }

    /// The gain of three high-passes at `hz` (44.1 kHz), measured on a settled sine.
    fn high_pass_gain(hz: f64) -> f64 {
        let n = 44_100 * 4;
        let mut x: Vec<f64> = (0..n)
            .map(|t| (TAU * hz * t as f64 / 44_100.0).sin())
            .collect();
        for _ in 0..3 {
            high_pass(&mut x);
        }
        x[n - 44_100..]
            .iter()
            .fold(0.0_f64, |peak, s| peak.max(s.abs()))
    }

    #[test]
    fn three_high_passes_are_3_db_down_near_27_hz() {
        assert!(
            (high_pass_gain(27.0) - 0.5_f64.sqrt()).abs() < 0.05,
            "{}",
            high_pass_gain(27.0)
        );
        assert!(high_pass_gain(1000.0) > 0.99);
        assert!(high_pass_gain(5.0) < 0.03);
    }

    #[test]
    fn frames_cover_the_loop_and_its_window_tail() {
        // n = floor((N + 143)/16).
        for (len, frames) in [(0, 8), (1, 9), (16, 9), (17, 10), (1000, 71)] {
            assert_eq!(detection(&vec![0.0; len]).len(), frames, "{len}");
        }
    }

    #[test]
    fn a_band_rises_by_k_times_its_ratio_and_falls_with_its_time_constant() {
        // Detector step 5's literals: (band, h, T ms, k, ε).
        for (band, hold, ms, k, floor) in [
            (&LOW, 55, 15.0_f64, 4.5, 1e-10 / 1.8),
            (&HIGH, 41, 10.0, 5.0, 1e-10 / 20.0),
        ] {
            let mut raw = vec![1.0; 10];
            raw.extend([2.0; 100]);
            let d = band.detect(&raw);
            assert_eq!(d[0], k * (1.0 / floor - 1.0), "from nothing: E/ε");
            assert_eq!(d[1..10], [0.0; 9]);
            assert_eq!(d[10], k * (2.0 - 1.0));
            assert!(d[11..].iter().all(|&d| d == 0.0));

            // 2 is held for h frames, then E falls by exp(−1/(2.7562·T)) per frame: a return to 1 after m frames of
            // falling rises from 2·fall^m.
            let m = 60;
            let mut raw = vec![2.0];
            raw.extend(vec![0.0; hold + m]);
            raw.push(1.0);
            let fall = (-1.0 / (2.7562 * ms)).exp();
            let want = k * (1.0 / (2.0 * fall.powi(m as i32)) - 1.0);
            let got = band.detect(&raw)[hold + m + 1];
            assert!((got - want).abs() < 1e-9, "{got} vs {want}");
        }
    }
}
