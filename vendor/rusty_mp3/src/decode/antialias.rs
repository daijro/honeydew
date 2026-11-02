//! Alias reduction.
//!
//! The hybrid filterbank introduces aliasing between adjacent subbands. Eight
//! butterflies per subband boundary cancel it, each an orthogonal rotation by
//! `(cs, ca)` derived from the eight `ci` coefficients (ISO Table B.9). Applied
//! to long blocks and the long part of mixed blocks; pure short blocks skip it.

use std::sync::OnceLock;

use crate::frame::{BlockType, GranuleSideInfo, GRANULE_LINES};

/// The eight alias-reduction `ci` coefficients (ISO 11172-3 Table B.9).
const CI: [f32; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];

/// `(cs, ca)` butterfly weights: `cs = 1/√(1+ci²)`, `ca = ci/√(1+ci²)`.
fn weights() -> &'static ([f32; 8], [f32; 8]) {
    static T: OnceLock<([f32; 8], [f32; 8])> = OnceLock::new();
    T.get_or_init(|| {
        let mut cs = [0f32; 8];
        let mut ca = [0f32; 8];
        for i in 0..8 {
            let d = (1.0 + CI[i] * CI[i]).sqrt();
            cs[i] = 1.0 / d;
            ca[i] = CI[i] / d;
        }
        (cs, ca)
    })
}

/// Apply the cross-subband alias-cancellation butterflies in place.
pub fn reduce(gi: &GranuleSideInfo, lines: &mut [f32; GRANULE_LINES]) {
    let is_short = gi.window_switching && gi.block_type == BlockType::Short;
    // Subband boundaries to butterfly: none for pure short, 1 for the long part
    // of a mixed block, all 31 for long blocks.
    let boundaries = if is_short {
        usize::from(gi.mixed_block)
    } else {
        31
    };
    let (cs, ca) = weights();
    for sb in 1..=boundaries {
        let base = sb * 18;
        // The butterfly touches `base-8 ..= base+7` and nothing else. Indexing
        // `lines` directly costs four bounds checks per butterfly -- two reads and
        // two writes -- because nothing proves `base + i < 576`; slicing the window
        // once per subband makes every index below constant-bounded.
        let w = &mut lines[base - 8..base + 8];
        for i in 0..8 {
            let (lower, upper) = (7 - i, 8 + i);
            let a = w[lower];
            let b = w[upper];
            w[lower] = a * cs[i] - b * ca[i];
            w[upper] = b * cs[i] + a * ca[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn butterfly_preserves_energy_long_block() {
        let mut lines = [0f32; GRANULE_LINES];
        for i in 10..26 {
            lines[i] = i as f32 * 0.1; // energy straddling the first boundary (18)
        }
        let before: f32 = lines.iter().map(|v| v * v).sum();
        reduce(&GranuleSideInfo::default(), &mut lines); // long block
        let after: f32 = lines.iter().map(|v| v * v).sum();
        // Orthogonal rotations conserve energy.
        assert!(
            (before - after).abs() < 1e-2,
            "before {before} after {after}"
        );
    }

    #[test]
    fn short_block_is_skipped() {
        let mut lines = [0f32; GRANULE_LINES];
        lines[17] = 1.0;
        lines[18] = 2.0;
        let snapshot = lines;
        let gi = GranuleSideInfo {
            window_switching: true,
            block_type: BlockType::Short,
            mixed_block: false,
            ..Default::default()
        };
        reduce(&gi, &mut lines);
        assert_eq!(
            lines, snapshot,
            "pure short blocks must not be alias-reduced"
        );
    }
}
