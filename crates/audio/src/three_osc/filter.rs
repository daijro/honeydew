//! The 3x Osc filter: eight types, the laws term by term. `c` and `r`
//! are the summed cutoff and resonance (param + offset + echo + mods + tracking), unclamped.

/// Param 110's eight types, in menu order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterType {
    #[default]
    FastLp,
    Lp,
    Bp,
    Hp,
    Bs,
    Lpx2,
    SvfLp,
    SvfLpx2,
}

impl FilterType {
    pub fn from_param(v: f32) -> Self {
        use FilterType::*;
        [FastLp, Lp, Bp, Hp, Bs, Lpx2, SvfLp, SvfLpx2][(v.clamp(0.0, 1.0) * 7.0).round() as usize]
    }
}

/// Fast LP f: 0 for c ≤ 0, 1 for c ≥ 1, else min(1, (601^c − 1)/600 · 44100/fs).
pub fn fast_lp_f(c: f32, fs: f32) -> f32 {
    if c <= 0.0 {
        0.0
    } else if c >= 1.0 {
        1.0
    } else {
        ((601f32.powf(c) - 1.0) / 600.0 * 44_100.0 / fs).min(1.0)
    }
}

/// Fast LP r′: 0 for r ≤ 0, else 1 + (1 − 601^(1 − 0.625·min(r, 1)))/600.
pub fn fast_lp_r(r: f32) -> f32 {
    if r <= 0.0 {
        0.0
    } else {
        1.0 + (1.0 - 601f32.powf(1.0 - 0.625 * r.min(1.0))) / 600.0
    }
}

/// Biquad ω0 = (44100/fs)·max(0.02, 0.0596(51^c − 1) + 0.02), capped at 3 rad.
pub fn biquad_w0(c: f32, fs: f32) -> f32 {
    (44_100.0 / fs * (0.0596 * (51f32.powf(c) - 1.0) + 0.02).max(0.02)).min(3.0)
}

/// Biquad Q = 0.1 for r ≤ 0, else 0.1 + 0.4(e^(2.45739·min(r, 1)) − 1) (max 4.37).
pub fn biquad_q(r: f32) -> f32 {
    if r <= 0.0 {
        0.1
    } else {
        0.1 + 0.4 * (2.457_39 * r.min(1.0)).exp_m1()
    }
}

/// SVF f = 0 for c < 0, else min(1, 0.04(e^(3.2581c) − 1)·44100/fs).
pub fn svf_f(c: f32, fs: f32) -> f32 {
    if c < 0.0 {
        0.0
    } else {
        (0.04 * (3.2581 * c).exp_m1() * 44_100.0 / fs).min(1.0)
    }
}

/// SVF q = 1 for r ≤ 0, 0.2 for r ≥ 1, else 1 − 0.8r.
pub fn svf_q(r: f32) -> f32 {
    if r <= 0.0 {
        1.0
    } else if r >= 1.0 {
        0.2
    } else {
        1.0 - 0.8 * r
    }
}

/// LPx2 output gain 1 − 0.046875·Q·(3.2 − ω0).
pub fn lpx2_gain(q: f32, w0: f32) -> f32 {
    1.0 - 0.046_875 * q * (3.2 - w0)
}

/// One control tick's coefficients (biquad and SVF values step per tick). The Fast LP's f is ramped by the voice
/// and passed to [`Filter::process`] per sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coefs {
    kind: FilterType,
    /// Normalized RBJ b0, b1, b2, a1, a2.
    b: [f32; 5],
    /// LPx2's output gain (1 otherwise).
    gain: f32,
    fast_r: f32,
    svf_f: f32,
    /// The two SVF stages' q (the second is 0.25 + 0.75q).
    svf_q: [f32; 2],
}

impl Coefs {
    pub fn new(kind: FilterType, c: f32, r: f32, fs: f32) -> Self {
        let mut k = Self {
            kind,
            gain: 1.0,
            ..Self::default()
        };
        match kind {
            FilterType::FastLp => k.fast_r = fast_lp_r(r),
            FilterType::SvfLp | FilterType::SvfLpx2 => {
                let q = svf_q(r);
                k.svf_f = svf_f(c, fs);
                k.svf_q = [q, 0.25 + 0.75 * q];
            }
            _ => {
                let (w0, q) = (biquad_w0(c, fs), biquad_q(r));
                let (sin, cos) = w0.sin_cos();
                let alpha = sin / (2.0 * q);
                let (b0, b1, b2) = match kind {
                    FilterType::Hp => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
                    // Constant 0 dB peak.
                    FilterType::Bp => (alpha, 0.0, -alpha),
                    FilterType::Bs => (1.0, -2.0 * cos, 1.0),
                    _ => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
                };
                let a0 = 1.0 + alpha;
                k.b = [
                    b0 / a0,
                    b1 / a0,
                    b2 / a0,
                    -2.0 * cos / a0,
                    (1.0 - alpha) / a0,
                ];
                if kind == FilterType::Lpx2 {
                    k.gain = lpx2_gain(q, w0);
                }
            }
        }
        k
    }
}

/// A voice's stereo filter state. A frame processed with another type's coefficients clears it first.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filter {
    kind: FilterType,
    /// Per channel: Fast LP (v, y), or the two stages' biquad (s1, s2) / SVF (low, band).
    state: [[[f32; 2]; 2]; 2],
}

impl Filter {
    /// Filters one stereo frame. `fast_f` is the Fast LP's current (ramped) f.
    pub fn process(&mut self, x: [f32; 2], k: &Coefs, fast_f: f32) -> [f32; 2] {
        if k.kind != self.kind {
            *self = Self {
                kind: k.kind,
                ..Self::default()
            };
        }
        let mut out = [0.0; 2];
        for (ch, &x) in x.iter().enumerate() {
            let [s0, s1] = &mut self.state[ch];
            out[ch] = match k.kind {
                FilterType::FastLp => {
                    // v = r′·v + f·(x − y); y += v.
                    s0[0] = k.fast_r * s0[0] + fast_f * (x - s0[1]);
                    s0[1] += s0[0];
                    s0[1]
                }
                FilterType::Lpx2 => k.gain * biquad(s1, &k.b, biquad(s0, &k.b, x)),
                FilterType::SvfLp => svf(s0, k.svf_f, k.svf_q[0], x),
                FilterType::SvfLpx2 => {
                    svf(s1, k.svf_f, k.svf_q[1], svf(s0, k.svf_f, k.svf_q[0], x))
                }
                _ => biquad(s0, &k.b, x),
            };
        }
        out
    }
}

/// Transposed direct form II.
fn biquad(s: &mut [f32; 2], b: &[f32; 5], x: f32) -> f32 {
    let y = b[0] * x + s[0];
    s[0] = b[1] * x - b[3] * y + s[1];
    s[1] = b[2] * x - b[4] * y;
    y
}

/// Chamberlin: low += f·band; out = low; band += f(x − low − q·band).
fn svf(s: &mut [f32; 2], f: f32, q: f32, x: f32) -> f32 {
    s[0] += f * s[1];
    s[1] += f * (x - s[0] - q * s[1]);
    s[0]
}
