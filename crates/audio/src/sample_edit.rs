//! The audio clip editor's sample maths (Waveform, Region menu): beat → frame, the
//! region ops on decoded frames and the waveform's peak columns. Pure, app thread.
//!
//! The ops take one `Vec<f32>` per channel (mono one, stereo two, equal lengths) and frame ranges `[s, e)`. The sample
//! is treated as followed by silence: ops that only change frames ignore the part of a range past the end, and ops that
//! insert material at a position past the end first fill the gap with silence, so the material lands where asked.

use std::ops::Range;

/// The Speed slot param in semitones: s = trunc(fma_f32(v, 48, −24)), one fused multiply-add in f32. Pad playback,
/// the Speed texts, an audio clip's source length (`CLHd`@28) and the
/// waveform draw all read it. At the steps v = f32(k/48) it is k − 24, except k = 16, 19, 22 (−7, −4, −1) and k = 25,
/// 28, 31, 34, 37, 40 (0, 3, 6, 9, 12, 15), where the single rounding lands just inside the integer; a multiply then
/// a subtract would round twice and give k − 24 everywhere.
pub fn speed_semitones(speed: f32) -> i32 {
    48.0f32.mul_add(speed, -24.0).trunc() as i32
}

/// The Speed slot param as a playback-rate ratio: exp2f(s · 1/12) in f32, with s = [`speed_semitones`] and 1/12 the
/// f32 1/12.
pub fn speed_ratio(speed: f32) -> f64 {
    let twelfth = 1.0f32 / 12.0;
    f64::from((speed_semitones(speed) as f32 * twelfth).exp2())
}

/// The sample frame under `beat`: trunc(trunc(beat·60·rate/tempo)·speed), `speed` being the ratio; before 0 it is 0.
/// A negative beat, speed or tempo, and a product that is not finite (NaN, infinity, a tempo of 0), give frame 0: never
/// a wrapped frame, nor `usize::MAX` from an infinity. A finite product past `usize::MAX` still saturates there.
/// A reversed sample is not mirrored here: reverse each channel, apply the op at these frames, reverse back. That is
/// the mirroring, exact for every op.
pub fn frame_at(beat: f64, rate: u32, tempo: f64, speed: f64) -> usize {
    let beat_frames = (beat.max(0.0) * 60.0 * f64::from(rate) / tempo)
        .trunc()
        .max(0.0);
    let frames = beat_frames * speed;
    if frames.is_finite() {
        frames as usize
    } else {
        0
    }
}

fn clamped(ch: &[Vec<f32>], r: Range<usize>) -> Range<usize> {
    let end = r.end.min(ch.first().map_or(0, Vec::len));
    r.start.min(end)..end
}

fn pad(ch: &mut [Vec<f32>], len: usize) {
    for c in ch.iter_mut().filter(|c| c.len() < len) {
        c.resize(len, 0.0);
    }
}

/// `Insert silence`: `len` zero frames at `at`.
pub fn insert_silence(ch: &mut [Vec<f32>], at: usize, len: usize) {
    if len == 0 {
        return;
    }
    pad(ch, at);
    for c in ch {
        c.splice(at..at, std::iter::repeat_n(0.0, len));
    }
}

/// `Clear`: zero the range.
pub fn clear(ch: &mut [Vec<f32>], r: Range<usize>) {
    let r = clamped(ch, r);
    for c in ch {
        c[r.clone()].fill(0.0);
    }
}

/// `Delete space`: remove the range.
pub fn delete_space(ch: &mut [Vec<f32>], r: Range<usize>) {
    let r = clamped(ch, r);
    for c in ch {
        c.drain(r.clone());
    }
}

/// `Trim selection`: keep only the range.
pub fn trim(ch: &mut [Vec<f32>], r: Range<usize>) {
    let r = clamped(ch, r);
    for c in ch {
        c.truncate(r.end);
        c.drain(..r.start);
    }
}

/// `Duplicate`: a copy of the range right after it.
pub fn duplicate(ch: &mut [Vec<f32>], r: Range<usize>) {
    copy_to(ch, r.clone(), r.end);
}

/// Whether `Copy/Move to playhead` applies: a non-empty range and p ≤ s or p ≥ e.
fn outside(r: &Range<usize>, p: usize) -> bool {
    r.start < r.end && (p <= r.start || p >= r.end)
}

/// `Copy to playhead`: insert a copy of the range at `p`; nothing when p is inside it.
pub fn copy_to(ch: &mut [Vec<f32>], r: Range<usize>, p: usize) {
    if !outside(&r, p) {
        return;
    }
    pad(ch, r.end.max(p));
    for c in ch {
        let copy = c[r.clone()].to_vec();
        c.splice(p..p, copy);
    }
}

/// `Move to playhead`: `Copy to playhead`, then remove the source (moved along when p ≤ s).
pub fn move_to(ch: &mut [Vec<f32>], r: Range<usize>, p: usize) {
    if !outside(&r, p) {
        return;
    }
    copy_to(ch, r.clone(), p);
    let len = r.len();
    delete_space(
        ch,
        if p <= r.start {
            r.start + len..r.end + len
        } else {
            r
        },
    );
}

/// `Normalize`: the range × 1/peak, the peak taken over every channel; nothing when the range is silent.
pub fn normalize(ch: &mut [Vec<f32>], r: Range<usize>) {
    let r = clamped(ch, r);
    let peak = ch
        .iter()
        .flat_map(|c| &c[r.clone()])
        .fold(0f32, |m, x| m.max(x.abs()));
    if peak > 0.0 {
        // Division, not × (1/peak): the peak frame lands exactly on ±1.0.
        for x in ch.iter_mut().flat_map(|c| &mut c[r.clone()]) {
            *x /= peak;
        }
    }
}

/// `Fade In`: frame k of the N in the range × k/N.
pub fn fade_in(ch: &mut [Vec<f32>], r: Range<usize>) {
    fade(ch, r, |g| g);
}

/// `Fade Out`: frame k of the N in the range × (1 − k/N).
pub fn fade_out(ch: &mut [Vec<f32>], r: Range<usize>) {
    fade(ch, r, |g| 1.0 - g);
}

/// N is the whole range's length, also when it runs past the end.
fn fade(ch: &mut [Vec<f32>], r: Range<usize>, gain: impl Fn(f64) -> f64) {
    let n = r.len();
    for c in ch {
        for (k, x) in c.iter_mut().skip(r.start).take(n).enumerate() {
            *x *= gain(k as f64 / n as f64) as f32;
        }
    }
}

/// Frames each pair of a waveform summary ([`summarize`]) covers.
pub const SUMMARY_FRAMES: usize = 256;

/// One channel's waveform summary: (min, max) over every [`SUMMARY_FRAMES`] frames, the last pair over what is left.
/// [`peaks`] reads the pairs of the blocks a column covers whole, not their frames.
pub fn summarize(samples: &[f32]) -> Vec<(f32, f32)> {
    (samples.chunks(SUMMARY_FRAMES))
        .map(|c| (c.iter()).fold((c[0], c[0]), |(a, b), &s| (a.min(s), b.max(s))))
        .collect()
}

/// The waveform's (min, max) per pixel column x, over the frames at positions in
/// [first + x·fpp, first + (x + 1)·fpp). A column narrower than a frame takes the frame under its left edge; a column
/// with no frame (before 0 or past the end) is (0, 0). `summary` is the channel's [`summarize`] (or empty): a column
/// reads the pairs of the blocks it covers whole and the frames of its two ends, so it costs at most
/// 2·[`SUMMARY_FRAMES`] frames and fpp/[`SUMMARY_FRAMES`] pairs whatever the zoom, for the same result as its frames.
pub fn peaks(
    samples: &[f32],
    summary: &[(f32, f32)],
    frames_per_px: f64,
    first: f64,
    width: usize,
) -> Vec<(f32, f32)> {
    let edge = |x: usize| first + x as f64 * frames_per_px;
    (0..width)
        .map(|x| {
            let (lo, hi) = (edge(x).ceil(), edge(x + 1).ceil());
            let (lo, hi) = if hi > lo {
                (lo, hi)
            } else {
                (edge(x).floor(), edge(x).floor() + 1.0)
            };
            let (lo, hi) = (
                lo.max(0.0) as usize,
                (hi.max(0.0) as usize).min(samples.len()),
            );
            if lo >= hi {
                return (0.0, 0.0);
            }
            let (b0, b1) = (lo.div_ceil(SUMMARY_FRAMES), hi / SUMMARY_FRAMES);
            let whole = summary.get(b0..b1).filter(|w| !w.is_empty());
            let ends = match whole {
                Some(_) => [lo..b0 * SUMMARY_FRAMES, b1 * SUMMARY_FRAMES..hi],
                None => [lo..hi, hi..hi],
            };
            let frames = ends
                .into_iter()
                .flat_map(|r| samples[r].iter().map(|&s| (s, s)));
            (frames.chain(whole.into_iter().flatten().copied()))
                .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
                .unwrap_or((0.0, 0.0))
        })
        .collect()
}
