//! GMSynth waveshapes: the 29 slots of the OSC `A`/`B` menus as spectra in the `.spectra` layout (magnitudes of
//! partials 1..=512, then their cos-phases), and the oscillator tables and lookup.
//!
//! Sine, Saw, Square and Triangle are math. The other 25 slots are our own designs; their menu names only say what
//! each one was drawn to evoke. `Wavetable::build` runs on the app thread (2 MB per shape: build only the selected
//! shapes, share them between instances). `table_index_for`, `Wavetable::table` and `read` are allocation-free.

use std::f32::consts::PI;
use std::f64::consts::TAU;

/// The menu's slot count, index 0–28.
pub const SLOTS: usize = 29;
/// Partials per spectrum.
pub const PARTIALS: usize = 512;
/// Tables per shape: table `H − 1` holds harmonics 1..=H.
pub const TABLES: usize = PARTIALS;
/// Samples per table.
pub const TABLE_LEN: usize = 1024;

/// The math slots in menu order.
pub const SAW: usize = 16;
pub const SINE: usize = 18;
pub const SQUARE: usize = 22;
pub const TRIANGLE: usize = 26;

/// The sine's magnitude at partial 1 in a `.spectra` file.
const SINE_MAGNITUDE: f32 = 0.5;
/// `partial` draws in a sine basis; the tables use cos-phases, and sin x = cos(x − π/2).
const SINE_TO_COS_PHASE: f32 = -PI / 2.0;
/// Ours, not a law: the designed slots are scaled to the square's RMS (π/4, its plateau) so the
/// menu plays at an even loudness. The engine itself never normalizes.
const DESIGN_RMS: f32 = PI / 4.0;
/// The index is the top `32 − INDEX_SHIFT` = 10 bits of the 32-bit phase; the dither fills the 22 bits below.
const INDEX_SHIFT: u32 = 32 - TABLE_LEN.trailing_zeros();
const DITHER_MASK: u32 = (1 << INDEX_SHIFT) - 1;

/// One waveshape in the `.spectra` layout: magnitudes (≥ 0) and cos-phases (radians, −π..π) of partials 1..=512.
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrum {
    pub mag: [f32; PARTIALS],
    pub phase: [f32; PARTIALS],
}

/// The spectrum of menu slot `slot`, or `None` past the menu: a missing shape, which silences the voice.
pub fn spectrum(slot: usize) -> Option<Spectrum> {
    if slot >= SLOTS {
        return None;
    }
    let mut spectrum = Spectrum {
        mag: [0.0; PARTIALS],
        phase: [0.0; PARTIALS],
    };
    for n in 1..=PARTIALS {
        let (mag, phase) = partial(slot, n);
        let phase = phase + SINE_TO_COS_PHASE + if mag < 0.0 { PI } else { 0.0 };
        spectrum.mag[n - 1] = mag.abs();
        spectrum.phase[n - 1] = (phase + PI).rem_euclid(2.0 * PI) - PI;
    }
    if ![SAW, SINE, SQUARE, TRIANGLE].contains(&slot) {
        let rms = (spectrum.mag.iter().map(|m| m * m).sum::<f32>() / 2.0).sqrt();
        spectrum.mag.iter_mut().for_each(|m| *m *= DESIGN_RMS / rms);
    }
    Some(spectrum)
}

/// Signed magnitude and sine-basis phase of partial `n` (from 1) of `slot`. Only the math slots follow a law; the
/// rest are our own designs, built from textbook spectra: pulse trains, formant bumps, low-pass
/// roll-offs, comb notches and seeded jitter.
fn partial(slot: usize, n: usize) -> (f32, f32) {
    let nf = n as f32;
    let odd = n % 2 == 1;
    let saw = 1.0 / nf;
    let square = if odd { saw } else { 0.0 };
    let pulse = |duty: f32| (PI * nf * duty).sin().abs() / nf;
    let bump = |centre: f32, width: f32| (-((nf - centre) / width).powi(2)).exp();
    let low_pass = |corner: f32| 1.0 / (1.0 + (nf / corner).powi(2));
    let random = |seed: u32| hash(seed, n);
    let jitter = |seed: u32| PI * (2.0 * hash(seed, n) - 1.0);
    let only = |partials: &[(usize, f32)]| partials.iter().find(|p| p.0 == n).map_or(0.0, |p| p.1);
    match slot {
        // 0 Beast: saw with a resonant hump.
        0 => (saw * (1.0 + 3.0 * bump(6.0, 2.5)), 0.0),
        // 1 Crunch: 30 % pulse, scattered phases.
        1 => (pulse(0.3), 0.5 * jitter(1)),
        // 2 Double Spike: two softened clicks a cycle, 0.35 apart.
        2 => (
            2.0 * (PI * nf * 0.35).cos() * low_pass(10.0),
            PI / 2.0 - PI * nf * 0.35,
        ),
        // 3 Digital: the saw's partials with at most two bits set.
        3 => (if n.count_ones() <= 2 { saw } else { 0.0 }, 0.0),
        // 4 Dirt: ragged saw.
        4 => (saw * (0.4 + 1.2 * random(4)), jitter(5)),
        // 5 Electro: 12.5 % pulse.
        5 => (pulse(0.125), 0.0),
        // 6 Filth: bright odd partials, half-strength even ones, scattered.
        6 => (if odd { nf.powf(-0.8) } else { 0.5 * saw }, jitter(6)),
        // 7 Fox: nasal, peaks at 3 and 9.
        7 => (saw * saw + 0.5 * bump(3.0, 0.8) + 0.2 * bump(9.0, 2.0), 0.0),
        // 8 Funky: wah peak at 4.
        8 => (saw * (0.3 + 2.0 * bump(4.0, 1.2)), 0.0),
        // 9 Grubby: dull and uneven.
        9 => (nf.powf(-1.5) * (0.5 + random(9)), 0.0),
        // 10 Keys: hollow, odd-led.
        10 => (
            if n == 1 {
                1.0
            } else if odd {
                0.6 * saw * saw
            } else {
                0.3 * nf.powf(-2.5)
            },
            0.0,
        ),
        // 11 Liquid: round, with turning phases.
        11 => (nf.powi(-3), PI / 2.0 * (n - 1) as f32),
        // 12 Ogre: fat square, peak at 5.
        12 => (square * (1.0 + 2.0 * bump(5.0, 1.5)) * low_pass(20.0), 0.0),
        // 13 Rhodes: fundamental, soft low partials and a tine at 7 and 14.
        13 => (
            only(&[
                (1, 1.0),
                (2, 0.35),
                (3, 0.12),
                (4, 0.05),
                (7, 0.18),
                (8, 0.06),
                (14, 0.04),
            ]),
            0.0,
        ),
        // 14 Saw Mean: square-leaning saw with a biting peak at 14.
        14 => (
            saw * if odd { 1.0 } else { 0.5 } * (1.0 + 2.0 * bump(14.0, 5.0)),
            0.0,
        ),
        // 15 Saw Retro: 24 partials.
        15 => (if n <= 24 { saw } else { 0.0 }, 0.0),
        // 16 Saw: 1/n.
        SAW => (saw, 0.0),
        // 17 Silk.
        17 => (nf.powf(-2.5), 0.0),
        // 18 Sine: partial 1.
        SINE => (only(&[(1, SINE_MAGNITUDE)]), 0.0),
        // 19 Smooth Sub.
        19 => (only(&[(1, 1.0), (2, 0.2), (3, 0.05)]), 0.0),
        // 20 Square Retro: up to partial 15.
        20 => (if n <= 15 { square } else { 0.0 }, 0.0),
        // 21 Square Smooth.
        21 => (square * low_pass(7.0), 0.0),
        // 22 Square: odd 1/n.
        SQUARE => (square, 0.0),
        // 23 Strat: a plucked string seen through a pickup comb.
        23 => ((PI * nf * 0.18).sin().abs() * nf.powf(-1.2), 0.0),
        // 24 String: bowed, uneven.
        24 => (
            (PI * nf * 0.09).sin().abs() * saw * (0.8 + 0.4 * random(24)),
            0.3 * jitter(25),
        ),
        // 25 Throat: dark vowel.
        25 => (saw * (0.25 + bump(3.0, 1.0) + 0.6 * bump(9.0, 2.0)), 0.0),
        // 26 Triangle: odd 1/n² with alternating sign.
        TRIANGLE => (
            if odd { saw * saw } else { 0.0 },
            if n % 4 == 3 { PI } else { 0.0 },
        ),
        // 27 Troll: growl with a loud octave.
        27 => (
            if n == 2 {
                0.6
            } else if odd {
                nf.powf(-1.2)
            } else {
                0.3 * saw * saw
            },
            0.0,
        ),
        // 28 Vocal: "ah", three formants.
        _ => (
            saw * (0.25 + bump(5.0, 1.5) + 0.7 * bump(12.0, 2.5) + 0.3 * bump(22.0, 3.0)),
            0.0,
        ),
    }
}

/// A fixed pseudo-random number in [0, 1) for partial `n` (an integer mix hash), so the designs never drift.
fn hash(seed: u32, n: usize) -> f32 {
    let mut x = (n as u32).wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x >> 8) as f32 / (1 << 24) as f32
}

/// One shape's 512 tables (2 MB), built on the app thread and shared by every instance that selects the shape.
pub struct Wavetable {
    tables: Box<[[f32; TABLE_LEN]]>,
}

impl Wavetable {
    /// Table `H − 1` is Σ_{h=1..H} mag_h · cos(phase_h + 2π·h·n/1024) for n in 0..1024, with no normalization.
    pub fn build(spectrum: &Spectrum) -> Self {
        let sine: Vec<f64> = (0..TABLE_LEN)
            .map(|i| (TAU * i as f64 / TABLE_LEN as f64).sin())
            .collect();
        let mut sum = vec![0.0_f64; TABLE_LEN];
        let mut tables = vec![[0.0_f32; TABLE_LEN]; TABLES].into_boxed_slice();
        for (h, table) in (1..).zip(tables.iter_mut()) {
            let mag = f64::from(spectrum.mag[h - 1]);
            if mag != 0.0 {
                // cos(φ + x) = cos φ · cos x − sin φ · sin x, with cos x read a quarter cycle ahead in the sine table.
                let phase = f64::from(spectrum.phase[h - 1]);
                let (a, b) = (mag * phase.cos(), mag * phase.sin());
                for (n, v) in sum.iter_mut().enumerate() {
                    let k = h * n;
                    *v += a * sine[(k + TABLE_LEN / 4) % TABLE_LEN] - b * sine[k % TABLE_LEN];
                }
            }
            for (dst, &src) in table.iter_mut().zip(&sum) {
                *dst = src as f32;
            }
        }
        Self { tables }
    }

    /// Table `index`, as `table_index_for` gives it; an index past the last reads the last.
    pub fn table(&self, index: usize) -> &[f32; TABLE_LEN] {
        &self.tables[index.min(TABLES - 1)]
    }
}

/// The table index (H − 1) for `freq` Hz at `sample_rate`, chosen once per 32-sample control block: H = 512 when
/// f ≤ sr/1024, else H = min(512, ⌊sr/(2f)⌋ + 1). The `+ 1` lets the top harmonic sit just above Nyquist, on
/// purpose. Choir and sync detuning don't change the choice.
/// The sign of `freq` is ignored, and sr ≤ 0 or a NaN or infinite input gives table 0
/// (H = 1).
pub fn table_index_for(freq: f32, sample_rate: f32) -> usize {
    let (f, sr) = (f64::from(freq).abs(), f64::from(sample_rate));
    if !f.is_finite() || !sr.is_finite() || sr <= 0.0 {
        return 0;
    }
    let h = if f <= sr / (2 * TABLES) as f64 {
        TABLES
    } else {
        (sr / (2.0 * f)).floor() as usize + 1
    };
    h.min(TABLES) - 1
}

/// One sample: the index is the top 10 bits of `phase` plus a uniform 22-bit `dither` (bits above 22 are
/// ignored), with no interpolation. Draw one dither per sample and share it between both oscillators and every choir
/// copy.
pub fn read(table: &[f32; TABLE_LEN], phase: u32, dither: u32) -> f32 {
    table[(phase.wrapping_add(dither & DITHER_MASK) >> INDEX_SHIFT) as usize]
}
