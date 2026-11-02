//! 3x Osc shapes (the wavetables and the Custom shape): 16384-point cycles read with linear
//! interpolation, and the HQ FFT mip levels. The fixed shapes are built once per process, off the audio thread, and
//! shared by every instance.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::sample::SampleData;

/// Points per raw cycle.
pub const CYCLE: usize = 16384;
/// HQ mip levels and their points per cycle.
pub const MIP_LEVELS: usize = 44;
pub const MIP_POINTS: usize = 32768;
/// Mip tables apply only when the step is above this (cycles per sample).
pub const MIP_MIN_STEP: f64 = 0.0005;
/// Noise ignores pitch: its fixed step in cycles per sample.
pub const NOISE_STEP: f64 = 0.000_292_20;
/// A Custom WAV is read up to this many frames.
pub const CUSTOM_MAX_FRAMES: usize = 65536;
/// The Custom resampler is a Blackman-windowed sinc with this many zero
/// crossings per side.
const SINC_ZEROS: f64 = 16.0;
/// The seed of the 16384 Noise randoms (xorshift32, uniform in −1..1).
const NOISE_SEED: u32 = 0x3c0f_fee5;

/// Params 1–3, in menu order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Sine,
    Triangle,
    Square,
    Saw,
    Analog,
    Noise,
    Custom,
}

impl Shape {
    pub fn from_param(v: f32) -> Self {
        use Shape::*;
        [Sine, Triangle, Square, Saw, Analog, Noise, Custom]
            [(v.clamp(0.0, 1.0) * 6.0).round() as usize]
    }

    /// HQ mip-maps every shape but Sine and Noise.
    pub fn has_mips(self) -> bool {
        !matches!(self, Shape::Sine | Shape::Noise)
    }
}

/// HQ mip level for an osc at `f` Hz: L = clamp(floor(n_eff/3 − 5), 0, 42), n_eff = 12·log2(f/8.17579892).
pub fn mip_level(f: f32) -> usize {
    let n_eff = 12.0 * (f as f64 / 8.175_798_92).log2();
    (n_eff / 3.0 - 5.0).floor().clamp(0.0, 42.0) as usize
}

/// The harmonics mip level `level` keeps: h ≤ 1024·2^(−L/4) (DC included).
pub fn harmonics(level: usize) -> usize {
    (1024.0 * 2f64.powf(-(level as f64) / 4.0)).floor() as usize
}

/// One shape: its raw cycle and, for HQ shapes, `MIP_LEVELS` × `MIP_POINTS` band-limited cycles.
pub struct Wave {
    raw: Box<[f32]>,
    mips: Box<[f32]>,
}

impl Wave {
    fn new(raw: Vec<f32>, mipped: bool) -> Self {
        let mips = if mipped { mip_levels(&raw) } else { Vec::new() };
        Self {
            raw: raw.into(),
            mips: mips.into(),
        }
    }

    /// The sample at `phase` (0..1): mip level `level` when there is one, else the raw cycle.
    #[inline]
    pub(super) fn read(&self, phase: f64, level: Option<u8>) -> f32 {
        match level {
            Some(l) if !self.mips.is_empty() => {
                let start = l as usize * MIP_POINTS;
                lerp(&self.mips[start..start + MIP_POINTS], phase)
            }
            _ => lerp(&self.raw, phase),
        }
    }

    /// The raw 16384-point cycle.
    pub fn raw(&self) -> &[f32] {
        &self.raw
    }

    /// The bytes the tables hold (the raw cycle and the mip levels): what a cache counts against its memory budget.
    pub fn byte_len(&self) -> usize {
        4 * (self.raw.len() + self.mips.len())
    }

    /// Builds a Custom shape from a WAV (app thread): its first 65536 frames,
    /// channels averaged, resampled (band-limited sinc) to one 16384-point cycle, then mip-mapped. `None` for an empty
    /// file.
    pub fn custom(sample: &SampleData) -> Option<Self> {
        let n = sample.left.len().min(CUSTOM_MAX_FRAMES);
        if n == 0 {
            return None;
        }
        let mono: Vec<f64> = match &sample.right {
            Some(right) => (0..n)
                .map(|i| {
                    (sample.left[i] as f64 + right.get(i).copied().unwrap_or(0.0) as f64) / 2.0
                })
                .collect(),
            None => sample.left[..n].iter().map(|&v| v as f64).collect(),
        };
        Some(Self::new(resample_cycle(&mono), true))
    }
}

/// The shared fixed shapes, Sine…Noise, then Custom's table before a file is loaded.
pub(super) struct Bank([Wave; 7]);

impl Bank {
    pub(super) fn get() -> &'static Self {
        static BANK: OnceLock<Bank> = OnceLock::new();
        BANK.get_or_init(|| {
            Bank(
                [
                    Shape::Sine,
                    Shape::Triangle,
                    Shape::Square,
                    Shape::Saw,
                    Shape::Analog,
                    Shape::Noise,
                    Shape::Custom,
                ]
                // Custom's zeros need no mips: a level falls back to the raw (zero) cycle.
                .map(|s| Wave::new(raw_cycle(s), s.has_mips() && s != Shape::Custom)),
            )
        })
    }

    /// `shape`'s wave; Custom resolves to `custom`, or silence when no file is loaded.
    pub(super) fn wave<'a>(&'a self, shape: Shape, custom: Option<&'a Wave>) -> &'a Wave {
        match (shape, custom) {
            (Shape::Custom, Some(custom)) => custom,
            (s, _) => &self.0[s as usize],
        }
    }
}

/// `shape`'s raw 16384-point cycle (Custom: the zeros it plays before a file is loaded).
pub fn cycle(shape: Shape) -> &'static [f32] {
    &Bank::get().wave(shape, None).raw
}

fn raw_cycle(shape: Shape) -> Vec<f32> {
    let mut seed = NOISE_SEED;
    (0..CYCLE)
        .map(|i| {
            let p = i as f64 / CYCLE as f64;
            (match shape {
                Shape::Sine => (2.0 * PI * p).sin(),
                Shape::Triangle => match p {
                    p if p < 0.25 => 4.0 * p,
                    p if p < 0.75 => 2.0 - 4.0 * p,
                    p => 4.0 * p - 4.0,
                },
                Shape::Square => {
                    if p < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Shape::Saw => 2.0 * p - 1.0,
                // A fresh
                // instance's custom table is all zeros, so Custom without a file is silent.
                Shape::Custom => 0.0,
                Shape::Analog => match p {
                    p if p < 0.46387 => 1.0 - 2.0 * 0.997_f64.powf(CYCLE as f64 * p),
                    p if p < 0.83333 => 1.0,
                    _ => -1.0,
                },
                Shape::Noise => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as f64 / u32::MAX as f64 * 2.0 - 1.0
                }
            }) as f32
        })
        .collect()
}

#[inline]
fn lerp(table: &[f32], phase: f64) -> f32 {
    let x = phase * table.len() as f64;
    let i = x as usize;
    let frac = (x - i as f64) as f32;
    let mask = table.len() - 1;
    let a = table[i & mask];
    a + (table[(i + 1) & mask] - a) * frac
}

/// The 44 band-limited levels of a 16384-point cycle, each 32768 points: one forward FFT, then one inverse FFT per
/// pair of levels (level a in the real part, level b in the imaginary part).
fn mip_levels(raw: &[f32]) -> Vec<f32> {
    let (mut re, mut im): (Vec<f64>, Vec<f64>) =
        (raw.iter().map(|&v| v as f64).collect(), vec![0.0; CYCLE]);
    fft(&mut re, &mut im, false);
    let mut out = vec![0.0f32; MIP_LEVELS * MIP_POINTS];
    let (mut zr, mut zi) = (vec![0.0; MIP_POINTS], vec![0.0; MIP_POINTS]);
    for pair in (0..MIP_LEVELS).step_by(2) {
        zr.fill(0.0);
        zi.fill(0.0);
        // Z = Ya + i·Yb, where Y[h] = 2·X[h] (the size doubles) and Y[N − h] = conj(Y[h]).
        for (level, real) in [(pair, true), (pair + 1, false)] {
            for h in 0..=harmonics(level) {
                let (xr, xi) = (2.0 * re[h], 2.0 * im[h]);
                let (ar, ai) = if real { (xr, xi) } else { (-xi, xr) }; // × i for level b
                zr[h] += ar;
                zi[h] += ai;
                if h > 0 {
                    let (cr, ci) = if real { (xr, -xi) } else { (xi, xr) }; // i·conj(Y)
                    zr[MIP_POINTS - h] += cr;
                    zi[MIP_POINTS - h] += ci;
                }
            }
        }
        fft(&mut zr, &mut zi, true);
        let scale = 1.0 / MIP_POINTS as f64;
        for n in 0..MIP_POINTS {
            out[pair * MIP_POINTS + n] = (zr[n] * scale) as f32;
            out[(pair + 1) * MIP_POINTS + n] = (zi[n] * scale) as f32;
        }
    }
    out
}

/// In-place radix-2 complex FFT (unscaled; `inverse` flips the exponent sign).
fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let n = re.len();
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
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        for k in 0..half {
            let (wi, wr) = (sign * 2.0 * PI * k as f64 / len as f64).sin_cos();
            for start in (0..n).step_by(len) {
                let (a, b) = (start + k, start + k + half);
                let tr = re[b] * wr - im[b] * wi;
                let ti = re[b] * wi + im[b] * wr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// The whole of `src` as one periodic cycle, resampled to `CYCLE` points by windowed sinc.
fn resample_cycle(src: &[f64]) -> Vec<f32> {
    let n = src.len();
    let ratio = n as f64 / CYCLE as f64;
    let cutoff = (1.0 / ratio).min(1.0);
    let half = (SINC_ZEROS / cutoff).ceil() as i64;
    (0..CYCLE)
        .map(|j| {
            let centre = j as f64 * ratio;
            let base = centre.floor() as i64;
            let mut acc = 0.0;
            for k in base - half + 1..=base + half {
                let t = (k as f64 - centre) * cutoff;
                if t.abs() >= SINC_ZEROS {
                    continue;
                }
                let sinc = if t == 0.0 {
                    1.0
                } else {
                    (PI * t).sin() / (PI * t)
                };
                let w = 0.42
                    + 0.5 * (PI * t / SINC_ZEROS).cos()
                    + 0.08 * (2.0 * PI * t / SINC_ZEROS).cos();
                acc += src[k.rem_euclid(n as i64) as usize] * cutoff * sinc * w;
            }
            acc as f32
        })
        .collect()
}
