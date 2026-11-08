// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A parameter's plain value (what the sound uses) and its normalised one (what the host stores), by place in the table
//! ([`crate::ids::SPECS`]). Pure: no audio, no files.

use nih_plug::prelude::{FloatRange, IntRange};

use crate::ids::{Kind, SPECS};

/// Parameter `i`'s plain value snapped to its range (discrete ones rounded).
pub fn clamp_plain(i: usize, v: f32) -> f32 {
    match SPECS[i].kind {
        Kind::Lin(lo, hi) | Kind::Skew(lo, hi, _) => {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                SPECS[i].default
            }
        }
        Kind::Int(lo, hi) => {
            if v.is_finite() {
                v.round().clamp(lo as f32, hi as f32)
            } else {
                SPECS[i].default
            }
        }
    }
}

fn float_range(lo: f32, hi: f32, skew: Option<f32>) -> FloatRange {
    match skew {
        None => FloatRange::Linear { min: lo, max: hi },
        Some(k) => FloatRange::Skewed {
            min: lo,
            max: hi,
            factor: FloatRange::skew_factor(k),
        },
    }
}

/// Parameter `i`'s plain value as the normalised one the host holds.
pub fn to_norm(i: usize, plain: f32) -> f32 {
    let v = clamp_plain(i, plain);
    match SPECS[i].kind {
        Kind::Lin(lo, hi) => float_range(lo, hi, None).normalize(v),
        Kind::Skew(lo, hi, k) => float_range(lo, hi, Some(k)).normalize(v),
        Kind::Int(lo, hi) => IntRange::Linear { min: lo, max: hi }.normalize(v as i32),
    }
}

/// The plain value of parameter `i` at normalised `norm`.
pub fn from_norm(i: usize, norm: f32) -> f32 {
    let n = norm.clamp(0.0, 1.0);
    match SPECS[i].kind {
        Kind::Lin(lo, hi) => float_range(lo, hi, None).unnormalize(n),
        Kind::Skew(lo, hi, k) => float_range(lo, hi, Some(k)).unnormalize(n),
        Kind::Int(lo, hi) => IntRange::Linear { min: lo, max: hi }.unnormalize(n) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_normalised_round_trip_for_every_parameter() {
        for (i, s) in SPECS.iter().enumerate() {
            for plain in [s.default, clamp_plain(i, -1.0e9), clamp_plain(i, 1.0e9)] {
                let back = from_norm(i, to_norm(i, plain));
                let tol = match s.kind {
                    Kind::Int(..) => 0.01,
                    Kind::Lin(lo, hi) | Kind::Skew(lo, hi, _) => (hi - lo) * 1e-3,
                };
                assert!((back - plain).abs() <= tol, "{}: {plain} -> {back}", s.id);
            }
        }
    }

    #[test]
    fn bad_values_fall_back_or_clamp() {
        assert_eq!(clamp_plain(0, f32::NAN), SPECS[0].default);
        assert_eq!(clamp_plain(0, 5.0), 1.0);
        assert_eq!(clamp_plain(crate::ids::POLY, 99.0), 16.0);
        assert_eq!(clamp_plain(crate::ids::POLY, 3.4), 3.0);
    }
}
