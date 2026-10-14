//! Mid/side joint-stereo encoding (the **R1+** enhancement) — the forward of
//! `decode/stereo.rs`.
//!
//! The decoder rotates `(M, S) → (L, R)` by `1/√2` in the frequency domain after
//! requantization. Because the analysis filterbank and MDCT are linear, doing the
//! same rotation in the **PCM domain** before analysis is exactly equivalent and
//! far simpler: store `M = (L+R)/√2`, `S = (L−R)/√2` and the decoder's inverse
//! reconstructs `L`/`R` to the bit. It is chosen per frame, only when the channels
//! are correlated enough that the side channel codes cheaply.

use std::f32::consts::FRAC_1_SQRT_2;

/// Mid/side transform of a frame's two channels into **caller-owned** buffers,
/// which are resized as needed and fully overwritten.
///
/// This is the form the encoder uses: it hands in scratch that persists across
/// frames, so a steady-state encode allocates nothing here. [`mid_side`] is the
/// owning convenience wrapper over it.
pub fn mid_side_into(left: &[f32], right: &[f32], mid: &mut Vec<f32>, side: &mut Vec<f32>) {
    let n = left.len().min(right.len());
    // `resize` only touches the tail, and every element in `..n` is written
    // below, so no zero-fill is paid on the steady-state path.
    mid.resize(n, 0.0);
    side.resize(n, 0.0);
    mid.truncate(n);
    side.truncate(n);
    for i in 0..n {
        mid[i] = (left[i] + right[i]) * FRAC_1_SQRT_2;
        side[i] = (left[i] - right[i]) * FRAC_1_SQRT_2;
    }
}

/// Mid/side transform of a frame's two channels: returns `[mid, side]`.
#[must_use]
pub fn mid_side(left: &[f32], right: &[f32]) -> Vec<Vec<f32>> {
    let (mut mid, mut side) = (Vec::new(), Vec::new());
    mid_side_into(left, right, &mut mid, &mut side);
    vec![mid, side]
}

/// The legacy raw-energy M/S heuristic — M/S concentrates energy into the mid
/// channel when the channels are positively correlated, leaving the side channel
/// small (cheap to code), so switch when the side energy is well below the mid.
/// Superseded by the perceptual-entropy cost test in `Mp3State::decide_stereo`
/// (this was too conservative); retained for the `MP3_STEREO=energy` A/B path.
#[must_use]
pub fn prefer_mid_side(left: &[f32], right: &[f32]) -> bool {
    let mut mid_e = 0f64;
    let mut side_e = 0f64;
    let n = left.len().min(right.len());
    for i in 0..n {
        let m = f64::from(left[i] + right[i]);
        let s = f64::from(left[i] - right[i]);
        mid_e += m * m;
        side_e += s * s;
    }
    // Strongly correlated: the side carries far less energy than the mid.
    side_e * 2.0 < mid_e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_side_inverts_to_left_right() {
        let l = [0.5f32, -0.3, 0.1, 0.8];
        let r = [0.4f32, -0.2, -0.1, 0.7];
        let ms = mid_side(&l, &r);
        for i in 0..4 {
            // Decoder rotation: L=(M+S)/√2, R=(M−S)/√2.
            let dl = (ms[0][i] + ms[1][i]) * FRAC_1_SQRT_2;
            let dr = (ms[0][i] - ms[1][i]) * FRAC_1_SQRT_2;
            assert!((dl - l[i]).abs() < 1e-6 && (dr - r[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn decision_tracks_correlation() {
        // Identical channels → side is zero → use M/S.
        let mono = vec![0.5f32; 1152];
        assert!(prefer_mid_side(&mono, &mono));
        // Anti-phase but otherwise identical → mid is zero, side large → no M/S.
        let anti: Vec<f32> = mono.iter().map(|x| -x).collect();
        assert!(!prefer_mid_side(&mono, &anti));
    }
}
