// Copyright (C) 2026 The Honeydew contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Honeydew (vendored addition): [`stretch`](super::stretch)'s API for the browser build, which cannot compile
//! Signalsmith Stretch (C++). It resamples instead: a stretched sound's pitch follows its speed, and the pitch factor
//! is not applied.
// ponytail: varispeed in place of a time stretch on the web; a pure-Rust stretcher would keep the pitch.

/// Linear interpolation of `x` (stride `step`, from `offset`) at frame position `pos`.
fn at(x: &[f32], step: usize, offset: usize, frames: usize, pos: f64) -> f32 {
    let i = pos.floor() as usize;
    let f = (pos - i as f64) as f32;
    let get = |k: usize| x[k.min(frames - 1) * step + offset];
    get(i) + (get(i + 1) - get(i)) * f
}

/// `left` / `right` resampled to `frames` frames (`rate` and `transpose` unused).
pub fn whole(left: &[f32], right: &[f32], _rate: u32, frames: usize, _transpose: f64) -> (Vec<f32>, Vec<f32>) {
    let n = left.len().min(right.len());
    if frames == 0 || n == 0 {
        return (vec![0.0; frames], vec![0.0; frames]);
    }
    let ratio = n as f64 / frames as f64;
    let side = |x: &[f32]| (0..frames).map(|k| at(x, 1, 0, n, k as f64 * ratio)).collect();
    (side(left), side(right))
}

/// The input of each call resampled to the output's length: no latency, the pitch following the speed.
pub struct Stream {
    rate: u32,
}

impl Stream {
    pub fn new(rate: u32) -> Stream {
        Stream { rate }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn lead(&self) -> usize {
        0
    }

    pub fn delay(&self) -> usize {
        0
    }

    pub fn history(&self) -> usize {
        0
    }

    pub fn interval(&self) -> usize {
        (f64::from(self.rate) * 0.03).round() as usize
    }

    pub fn start(&mut self, _ahead: &[f32], _speed: f64) {}

    pub fn set_transpose(&mut self, _factor: f64) {}

    /// Writes `input` (interleaved stereo) resampled to `left.len()` frames.
    pub fn process(&mut self, input: &[f32], left: &mut [f32], right: &mut [f32]) {
        let (n, frames) = (left.len().min(right.len()), input.len() / 2);
        if frames == 0 {
            left[..n].fill(0.0);
            right[..n].fill(0.0);
            return;
        }
        let ratio = frames as f64 / n.max(1) as f64;
        for k in 0..n {
            left[k] = at(input, 2, 0, frames, k as f64 * ratio);
            right[k] = at(input, 2, 1, frames, k as f64 * ratio);
        }
    }
}
