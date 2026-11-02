//! The DSP chain, tool only. Audio is one `Vec<f32>` per channel at the source rate; decoding is
//! `honeydew_audio::sample::decode` and encoding `FlacEncoder::push_i16` with [`dither`]'s output.

pub mod looping;
pub mod resample;

/// One microphone's take for [`premix`]: mono or stereo, a gain in dB and a balance from −1 (left) to 1 (right).
pub struct Mic<'a> {
    pub channels: &'a [Vec<f32>],
    pub gain_db: f32,
    pub pan: f32,
}

/// Sums time-aligned takes into one stereo pair as long as the longest (a shorter take ends in silence). A mono take
/// feeds both sides. Pan is a balance: the far side is scaled by 1 − |pan|, the near side kept, so centre is unity.
pub fn premix(mics: &[Mic]) -> Vec<Vec<f32>> {
    let len = mics
        .iter()
        .flat_map(|m| m.channels)
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    let mut out = vec![vec![0.0; len]; 2];
    for mic in mics {
        let gain = db(mic.gain_db);
        let sides = [(1.0 - mic.pan).min(1.0), (1.0 + mic.pan).min(1.0)];
        for (side, (out, balance)) in out.iter_mut().zip(sides).enumerate() {
            let Some(take) = mic.channels.get(side).or(mic.channels.first()) else {
                continue;
            };
            let g = gain * balance;
            out.iter_mut().zip(take).for_each(|(o, &x)| *o += g * x);
        }
    }
    out
}

/// Folds a stereo pair to its mid, (L + R) / 2, when the side's RMS is under −40 dB re the mid's. Anything else comes
/// back as it was.
pub fn fold_mono(channels: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let [l, r] = channels.as_slice() else {
        return channels;
    };
    let energy = |sign: f32| -> f64 {
        l.iter()
            .zip(r)
            .map(|(&a, &b)| f64::from(a + sign * b).powi(2))
            .sum()
    };
    // −40 dB in amplitude is 1e-4 in energy.
    if energy(-1.0) <= 1e-4 * energy(1.0) {
        vec![l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect()]
    } else {
        channels
    }
}

/// Cuts the lead below −80 dB re the peak, keeping 1 ms before the first frame above it.
pub fn trim_lead(channels: &mut [Vec<f32>], rate: u32) {
    let threshold = peak(channels) * db(-80.0);
    let first = (0..frames(channels)).find(|&i| loud(channels, i, threshold));
    let start = first.unwrap_or(0).saturating_sub(ms(rate, 1.0));
    channels.iter_mut().for_each(|c| drop(c.drain(..start)));
}

/// Keeps the audio up to the last frame above `floor_db` re the peak plus `fade_ms`, and `max_s` at most; the kept
/// audio's last `fade_ms` fade to exactly 0 on a raised cosine. So the fade falls after the last loud frame unless the
/// audio or `max_s` ends sooner.
pub fn trim_tail(
    channels: &mut [Vec<f32>],
    rate: u32,
    floor_db: f32,
    fade_ms: f32,
    max_s: Option<f32>,
) {
    let n = frames(channels);
    let threshold = peak(channels) * db(floor_db);
    let last = (0..n)
        .rev()
        .find(|&i| loud(channels, i, threshold))
        .map_or(0, |i| i + 1);
    let fade = ms(rate, fade_ms);
    let cap = max_s.map_or(n, |s| ms(rate, 1000.0 * s));
    let end = (last + fade).min(n).min(cap);
    let fade = fade.min(end);
    for c in channels.iter_mut() {
        c.truncate(end);
        for (i, x) in c[end - fade..].iter_mut().enumerate() {
            *x *= 0.5 * (1.0 + (std::f32::consts::PI * (i + 1) as f32 / fade as f32).cos());
        }
    }
}

/// Seeded TPDF dither to 16 bits: x × 32768 plus the sum of two uniform values in [−½, ½) LSB, rounded and held to
/// the i16 range (the decoder reads a sample back as q / 32768). The same seed gives the same samples.
pub fn dither(channels: &[Vec<f32>], seed: u64) -> Vec<Vec<i16>> {
    // xorshift64, which must not start at its fixed point 0.
    let mut state = (seed ^ 0x9E37_79B9_7F4A_7C15).max(1);
    let mut uniform = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let mut quantize = |x: f32| {
        let q = f64::from(x) * 32_768.0 + uniform() + uniform();
        q.round().clamp(-32_768.0, 32_767.0) as i16
    };
    channels
        .iter()
        .map(|c| c.iter().map(|&x| quantize(x)).collect())
        .collect()
}

/// Linear gain of `db` decibels.
pub(crate) fn db(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Frames in `ms` milliseconds, rounded.
fn ms(rate: u32, ms: f32) -> usize {
    (f64::from(rate) * f64::from(ms) / 1000.0).round() as usize
}

/// Frames every channel has.
fn frames(channels: &[Vec<f32>]) -> usize {
    channels.iter().map(Vec::len).min().unwrap_or(0)
}

/// The largest magnitude in any channel.
pub(crate) fn peak(channels: &[Vec<f32>]) -> f32 {
    channels.iter().flatten().fold(0.0, |p, x| p.max(x.abs()))
}

/// Whether any channel's frame `i` is above `threshold`.
fn loud(channels: &[Vec<f32>], i: usize, threshold: f32) -> bool {
    channels.iter().any(|c| c[i].abs() > threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premix_fold_trim_fade_and_dither() {
        // At 1 kHz a millisecond is a frame. 20 silent frames, 100 at 0.5, 100 at −40 dB, 200 at −66 dB (re 0.5).
        let take: Vec<f32> = [(20, 0.0), (100, 0.5), (100, 0.005), (200, 0.000_25)]
            .into_iter()
            .flat_map(|(n, x)| std::iter::repeat_n(x, n))
            .collect();
        let close = [take.clone()];
        let room = [take.clone(), take.clone()];
        let mics = |pan| {
            [
                Mic {
                    channels: &close,
                    gain_db: 0.0,
                    pan,
                },
                Mic {
                    channels: &room,
                    gain_db: -20.0 * 2f32.log10(),
                    pan: 0.0,
                },
            ]
        };
        // Centred, the pair is L = R = 1.5 × the take, and folds; panned hard left it stays stereo.
        let pair = premix(&mics(0.0));
        for side in &pair {
            assert!(
                side.iter()
                    .zip(&take)
                    .all(|(&o, &x)| (o - 1.5 * x).abs() < 1e-6)
            );
        }
        assert_eq!(fold_mono(premix(&mics(-1.0))).len(), 2);
        let mut mono = fold_mono(pair);
        assert_eq!(mono.len(), 1);

        trim_lead(&mut mono, 1_000);
        trim_tail(&mut mono, 1_000, -60.0, 30.0, None);
        // The lead keeps 1 frame of silence; the tail ends 30 frames after the last frame above −60 dB, which the
        // fade leaves alone and takes the frames after it to exactly 0.
        let x = &mono[0];
        assert_eq!(x.len(), 1 + 200 + 30);
        assert_eq!(x[0], 0.0);
        assert!((x[1] - 0.75).abs() < 1e-6 && (x[200] - 0.0075).abs() < 1e-8);
        assert!(x[201] < 0.000_375 && x[201] > 0.000_37 && x[215] < 0.000_2);
        assert_eq!(x[230], 0.0);
        let mut capped = mono.clone();
        trim_tail(&mut capped, 1_000, -60.0, 30.0, Some(0.1));
        assert_eq!((capped[0].len(), capped[0][99]), (100, 0.0));

        // Dither: the same seed gives the same samples, another seed others; all within 1.5 LSB of x × 32768.
        let a = dither(&mono, 7);
        assert_eq!(a, dither(&mono, 7));
        assert_ne!(a, dither(&mono, 8));
        assert!(
            a[0].iter()
                .zip(x)
                .all(|(&q, &x)| (f64::from(q) - f64::from(x) * 32_768.0).abs() < 1.5)
        );
    }
}
