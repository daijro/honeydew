//! Sustain loops for organs, pads and other held sounds.

/// Frames compared on each side of the loop points.
const SEAM: usize = 128;
/// End points tried: the last rising zero crossings before `max_end`.
const ENDS: usize = 32;

/// A loop `[start, end)` for a held sound: at least `min_len` frames, ending by `max_end`, starting after the attack
/// (the first frame at half the peak before `max_end`: a slow swell may peak after the cut) and after `xfade` frames
/// (room for [`bake_loop`]'s crossfade). Both points are rising zero crossings of the channels' sum (the first frame
/// above 0 after one at or below it), so their slopes match in sign, and `start` curves no more than three in four
/// frames of the search (its channels' summed |second difference|): once baked, the wrap's bend is the curvature at
/// `start`, and on a wide pair one side can peak where the sum crosses zero. (The lower quarter, not the median: a
/// decaying loop curves less than the search around it.) Of the pairs, the one whose 256 frames around `start` best
/// match those around `end` (least squared difference) wins. `None` when no pair fits.
// ponytail: every start against the last 32 ends, O(crossings × 32 × 256); decimate the search if noisy sources make
// it slow.
pub fn find_loop(
    channels: &[Vec<f32>],
    min_len: usize,
    max_end: usize,
    xfade: usize,
) -> Option<(usize, usize)> {
    let n = channels.iter().map(Vec::len).min()?;
    let sum: Vec<f32> = (0..n)
        .map(|i| channels.iter().map(|c| c[i]).sum())
        .collect();
    let peak = sum[..max_end.min(n)]
        .iter()
        .fold(0f32, |p, x| p.max(x.abs()));
    let attack = sum.iter().position(|x| x.abs() >= 0.5 * peak)?;
    let rising: Vec<usize> = (1..n.saturating_sub(SEAM))
        .filter(|&i| sum[i - 1] <= 0.0 && sum[i] > 0.0)
        .collect();
    let first = attack.max(xfade).max(SEAM);
    let curve = |i: usize| -> f32 {
        (channels.iter())
            .map(|c| (c[i] - 2.0 * c[i - 1] + c[i - 2]).abs())
            .sum()
    };
    let mut curves: Vec<f32> = (first..max_end.min(n)).map(curve).collect();
    let quarter = curves.len().checked_sub(1)? / 4;
    let (_, &mut low, _) = curves.select_nth_unstable_by(quarter, f32::total_cmp);
    let starts: Vec<usize> = (rising.iter().copied())
        .filter(|&s| s >= first && curve(s) <= low)
        .collect();
    let mut best: Option<(f64, usize, usize)> = None;
    for &end in rising.iter().rev().filter(|&&e| e <= max_end).take(ENDS) {
        for &start in starts.iter().take_while(|&&s| s + min_len <= end) {
            let window = |at: usize| &sum[at - SEAM..at + SEAM];
            let error = window(start)
                .iter()
                .zip(window(end))
                .map(|(&a, &b)| f64::from(a - b).powi(2))
                .sum();
            if best.is_none_or(|(least, ..)| error < least) {
                best = Some((error, start, end));
            }
        }
    }
    best.map(|(_, start, end)| (start, end))
}

/// Bakes an equal-gain crossfade (gains summing to 1) into the `xfade` frames before `end` (held to the loop's length
/// and to `start`), from the audio there to the audio before `start`, so the last frame is the one before `start` and
/// the wrap continues it. Then cuts the sample at `end`: past it, nothing plays while looping. Equal gain, because the
/// search picks well-correlated sides, where an equal-power fade would swell by up to 3 dB on every pass.
pub fn bake_loop(channels: &mut [Vec<f32>], start: usize, end: usize, xfade: usize) {
    let xfade = xfade.min(start).min(end - start);
    for c in channels.iter_mut() {
        for i in 0..xfade {
            let t = (i + 1) as f32 / xfade as f32 * std::f32::consts::FRAC_PI_2;
            let (out, into) = (t.cos().powi(2), t.sin().powi(2));
            c[end - xfade + i] = c[end - xfade + i] * out + c[start - xfade + i] * into;
        }
        c.truncate(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    #[test]
    fn a_vibrato_sine_loops_with_its_seam_under_minus_50_db() {
        // Middle C with a 5.3 Hz, ±0.5 % vibrato for 3 s: no whole number of frames repeats it, so only a search over
        // many pairs gets under −50 dB (this one finds about −77 dB).
        let rate = 44_100.0;
        let x: Vec<f32> = (0..132_300)
            .map(|i| {
                let t = f64::from(i) / rate;
                let phase = TAU * 261.63 * t - 0.005 * 261.63 / 5.3 * (TAU * 5.3 * t).cos();
                (0.5 * phase.sin()) as f32
            })
            .collect();
        let (min_len, max_end, xfade) = (22_050, 88_200, 1_764); // 0.5 s, 2 s, 40 ms
        let (start, end) =
            find_loop(std::slice::from_ref(&x), min_len, max_end, xfade).expect("a loop");
        assert!(
            end - start >= min_len && end <= max_end && start >= xfade,
            "{start}..{end}"
        );
        let window = |at: usize| &x[at - 128..at + 128];
        let error: f64 = window(start)
            .iter()
            .zip(window(end))
            .map(|(&a, &b)| f64::from(a - b).powi(2))
            .sum();
        let energy: f64 = window(end).iter().map(|&b| f64::from(b).powi(2)).sum();
        let db = 10.0 * (error / energy).log10();
        assert!(db < -50.0, "seam {db} dB");

        // Baked: cut at the end, untouched before the crossfade, and its last frame is the one before the start, so the
        // wrap to the start continues the audio.
        let mut baked = [x.clone()];
        bake_loop(&mut baked, start, end, xfade);
        let y = &baked[0];
        assert_eq!(y.len(), end);
        assert_eq!(y[..end - xfade], x[..end - xfade]);
        assert!((y[end - 1] - x[start - 1]).abs() < 1e-6);
        // The two sides are correlated (that is what the search picks), so the fade must not swell past the source's
        // 0.5 peak: an equal-power fade reaches about +3 dB mid-way, on every pass of the loop.
        let peak = y[end - xfade..].iter().fold(0f32, |p, v| p.max(v.abs()));
        assert!(peak <= 0.5 + 1e-4, "crossfade peak {peak}");
    }

    #[test]
    fn a_swell_peaking_after_the_kept_part_still_loops() {
        // A slow bowed swell (a soft violin section): 220 Hz rising from 0.05 to 1.0 over 10 s, kept for 2.5 s. Its
        // peak lies past the cut, where half of it would bar every start; the attack is half the kept part's peak.
        let x: Vec<f32> = (0..441_000)
            .map(|i| {
                let t = f64::from(i) / 44_100.0;
                ((0.05 + 0.095 * t) * (TAU * 220.0 * t).sin()) as f32
            })
            .collect();
        let (start, end) =
            find_loop(std::slice::from_ref(&x), 44_100, 110_250, 4_410).expect("a loop");
        assert!(end - start >= 44_100 && end <= 110_250, "{start}..{end}");
    }

    #[test]
    fn a_wide_stereo_loop_bends_at_its_seam_no_more_than_twice_its_median() {
        // A wide pair: a rich tone left and another a fifth up right, each with its own vibrato, so where their sum
        // crosses zero either side may be curving hard. The wrap's bend (how far the start frame strays from the line
        // through the two frames before the end) must stay near the loop's median bend; unfiltered starts gave 5.8×.
        let rate = 44_100.0;
        let side = |hz: f64, vibrato: f64, i: u32| {
            let t = f64::from(i) / rate;
            let phase = hz * t - 0.006 * hz / vibrato * (TAU * vibrato * t).cos();
            (1..8)
                .map(|h| 0.3 / f64::from(h) * (TAU * f64::from(h) * phase + f64::from(h)).sin())
                .sum::<f64>() as f32
        };
        let pair: Vec<Vec<f32>> = [(164.0, 4.7), (246.0, 5.9)]
            .iter()
            .map(|&(hz, vibrato)| (0..132_300).map(|i| side(hz, vibrato, i)).collect())
            .collect();
        let xfade = 2_646; // 60 ms
        let (start, end) = find_loop(&pair, 44_100, 110_250, xfade).expect("a loop");
        let mut baked = pair.clone();
        bake_loop(&mut baked, start, end, xfade);
        let bend = |at: usize, before: usize| -> f32 {
            (baked.iter())
                .map(|c| (c[at] - 2.0 * c[before] + c[before - 1]).abs())
                .sum()
        };
        let mut bends: Vec<f32> = (start + 2..end).map(|i| bend(i, i - 1)).collect();
        bends.sort_by(f32::total_cmp);
        let (seam, median) = (bend(start, end - 1), bends[bends.len() / 2]);
        assert!(seam <= 2.0 * median, "seam bend {seam} vs median {median}");
    }
}
