//! MiniSynth (ID 0).

use std::f32::consts::TAU;
use std::sync::OnceLock;

/// MiniSynth has 5 voices.
pub const VOICES: usize = 5;
/// The control block: pitch, glide, drift and the other block-rate laws are evaluated every 32 frames.
const BLOCK: usize = 32;
/// LFO Rate and Phaser Speed periods in beats, picked by trunc(11·v) (the DSP plays "1 bar" as 16
/// beats, "2 bars" 64, "4 bars" 128, "16 bars" 256).
const RATE_BEATS: [f64; 12] = [
    0.25, 0.375, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0, 64.0, 128.0, 256.0,
];
/// The chorus LFO phases (L, R) at the start and whenever it is switched on.
const CHORUS_PHASES: [f32; 2] = [0.62, 0.1];
/// Voice states below this are zeroed once per block, so a decay toward 0 never runs on subnormals (ours, far below
/// hearing; as GMSynth's).
const FLUSH: f32 = 1e-20;
/// A key that is not held, in [`MiniSynth::held`].
const NOT_HELD: f32 = -1.0;
/// Samples per base-wave table, and the number of tables (one per harmonic count).
const TABLE: usize = 2048;
const HARMONICS: usize = 1024;
/// SUPERSAW: 8 oscillators at f·(1 + m·c).
const SUPERSAW_SPREAD: [f32; 8] = [-0.05, -0.0375, -0.025, -0.0125, 0.0, 0.0125, 0.025, 0.0375];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParameterSpec {
    pub name: &'static str,
    pub default: f32,
    pub steps: usize,
}

pub const PARAMETERS: [ParameterSpec; 44] = [
    ParameterSpec {
        name: "Enable",
        default: 1.0,
        steps: 2,
    },
    ParameterSpec {
        name: "Voices Mode",
        default: 0.0,
        steps: 3,
    },
    ParameterSpec {
        name: "Transpose",
        default: 0.5,
        steps: 25,
    },
    ParameterSpec {
        name: "Slide Time",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "MW Dest",
        default: 0.0,
        steps: 3,
    },
    ParameterSpec {
        name: "MW Amount",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "OSC Kind",
        default: 0.0,
        steps: 10,
    },
    ParameterSpec {
        name: "OSC Mod",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "OSC Noise",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "FLT Freq",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "FLT Peak",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "FLT Kbd",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "FLT Vel",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "EGF Atk",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "EGF Dec",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "EGF Sus",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "EGF Rel",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "EGF Amnt",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "LFO Shape",
        default: 0.0,
        steps: 3,
    },
    ParameterSpec {
        name: "LFO Dest",
        default: 0.0,
        steps: 3,
    },
    ParameterSpec {
        name: "LFO Rate",
        default: 0.25,
        steps: 12,
    },
    ParameterSpec {
        name: "LFO Amnt",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "AMP Atk",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "AMP Dec",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "AMP Sus",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "AMP Rel",
        default: 0.07,
        steps: 0,
    },
    ParameterSpec {
        name: "DIST",
        default: 0.0,
        steps: 2,
    },
    ParameterSpec {
        name: "Overdrive",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "Decimator",
        default: 0.0,
        steps: 0,
    },
    ParameterSpec {
        name: "DELAY",
        default: 0.0,
        steps: 2,
    },
    ParameterSpec {
        name: "DELAY Time",
        default: 0.7,
        steps: 4,
    },
    ParameterSpec {
        name: "DELAY Fdbk",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "DELAY Filter",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "DELAY Mix",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "CHORUS",
        default: 0.0,
        steps: 2,
    },
    ParameterSpec {
        name: "CHORUS ModRate",
        default: 0.25,
        steps: 0,
    },
    ParameterSpec {
        name: "CHORUS ModDepth",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "CHORUS Mix",
        default: 0.5,
        steps: 0,
    },
    ParameterSpec {
        name: "PHASER",
        default: 0.0,
        steps: 2,
    },
    ParameterSpec {
        name: "PHASER ModSpeed",
        default: 0.1122,
        steps: 12,
    },
    ParameterSpec {
        name: "PHASER ModWidth",
        default: 0.4,
        steps: 0,
    },
    ParameterSpec {
        name: "PHASER Fdbk",
        default: 0.1,
        steps: 0,
    },
    ParameterSpec {
        name: "PHASER Mix",
        default: 1.0,
        steps: 0,
    },
    ParameterSpec {
        name: "PHASER Freq",
        default: 0.5,
        steps: 0,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniSynthError {
    InvalidSampleRate,
    InvalidValue,
    InvalidParameter(usize),
    MismatchedBuffers,
}

/// The Transpose step (p2): s = trunc(fma_f32(clamp(v, 0, 1), 24, −12)), one rounding, so k = 8, 11, 14, 17, 20 of
/// the 25 menu values sit one step off. The `%d` text uses the same law.
pub fn transpose_steps(v: f32) -> i32 {
    v.clamp(0.0, 1.0).mul_add(24.0, -12.0) as i32
}

/// The 1024 band-limited single-cycle tables T_H(n) = Σ_{h=1..H} M_h·cos(φ_h + 2π·h·n/2048), H = 1…1024, table H at
/// `(H − 1)·2048`. Built once, on the app thread, by the first [`MiniSynth::new`].
fn tables() -> &'static [f32] {
    static TABLES: OnceLock<Box<[f32]>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let turn = |n: usize| std::f64::consts::TAU * n as f64 / TABLE as f64;
        let (cos, sin): (Vec<f64>, Vec<f64>) = (0..TABLE)
            .map(|n| turn(n).sin_cos())
            .map(|(s, c)| (c, s))
            .unzip();
        let mut sum = vec![0.0f64; TABLE];
        let mut out = vec![0.0f32; HARMONICS * TABLE];
        for (h, table) in (1..=HARMONICS).zip(out.chunks_exact_mut(TABLE)) {
            let (magnitude, phase) = harmonic(h);
            let (a, b) = (magnitude * phase.cos(), magnitude * phase.sin());
            for (n, (acc, x)) in sum.iter_mut().zip(table).enumerate() {
                let i = h * n % TABLE;
                *acc += a * cos[i] - b * sin[i];
                *x = *acc as f32;
            }
        }
        out.into_boxed_slice()
    })
}

/// Harmonic h of the base wave, (M_h, φ_h): the closed form that reproduces the shipped spectrum within 0.005.
fn harmonic(h: usize) -> (f64, f64) {
    let h = h as f64;
    let magnitude = (0.1402
        + 0.1398 * (-(h - 1.0) / 2.8832).exp()
        + 0.0215 * (-(h - 1.0) / 9.4539).exp()
        + 0.0768 * (h / 1024.0).powf(2.4706))
        / h;
    let phase = -1.57007 + 0.00154 * (h - 1.0) - 3.09168 * h / (h * h + 14.42413);
    (magnitude, phase)
}

/// The table a voice reads, from its first oscillator's frequency: all 1024 harmonics up to sr/2048, else
/// clamp(floor(sr/(2·f₁)) + 1, 1, 1024).
fn harmonics(f1: f32, sample_rate: f32) -> usize {
    if f1 <= sample_rate / 2048.0 {
        HARMONICS
    } else {
        ((sample_rate / (2.0 * f1)).floor() as usize + 1).clamp(1, HARMONICS)
    }
}

/// The Modifier interval r = 2^(s/12 + 0.00625), s = trunc(double(fma_f32(m, 2, −1))·12).
fn interval(m: f32) -> f32 {
    let steps = (f64::from(m.mul_add(2.0, -1.0)) * 12.0).trunc() as f32;
    (steps / 12.0 + 0.00625).exp2()
}

/// `x` cycles as a 32-bit phase, wrapped.
fn cycles(x: f32) -> u32 {
    (f64::from(x) * 4_294_967_296.0) as u64 as u32
}

/// The phase increment trunc(f·2³²/sr).
fn increment(frequency: f32, sample_rate: f32) -> u32 {
    (f64::from(frequency) * 4_294_967_296.0 / f64::from(sample_rate)) as u32
}

/// The ladder low-pass: s1 += g(u − k·s4 − s1), s2 += g(s1 − s2), s3 += g(s2 − s3), s4 += g(s3 − s4)
/// as 4096 Euler substeps per sample with u held, then s1 := s1/(1 + s1²); the output is s4. The substeps are linear
/// in [s1, s2, s3, s4, u], so the 4096 of them are one matrix, the 4096th power of one substep, rebuilt with the
/// coefficients every block: the same map as the loop, up to rounding, for 20 multiply-adds a sample.
#[derive(Clone, Copy, Default)]
struct Ladder {
    state: [f32; 4],
    step: [[f32; 5]; 4],
}

impl Ladder {
    fn set(&mut self, g: f64, k: f64) {
        let mut m = [[0.0f64; 5]; 5];
        m[0] = [1.0 - g, 0.0, 0.0, -g * k, g];
        for i in 1..4 {
            m[i] = m[i - 1].map(|x| g * x);
            m[i][i] += 1.0 - g;
        }
        m[4][4] = 1.0;
        for _ in 0..12 {
            m = std::array::from_fn(|i| {
                std::array::from_fn(|j| (0..5).map(|k| m[i][k] * m[k][j]).sum())
            });
        }
        for (out, row) in self.step.iter_mut().zip(m) {
            *out = row.map(|x| x as f32);
        }
    }

    fn tick(&mut self, u: f32) -> f32 {
        let s = self.state;
        let mut next = self
            .step
            .map(|r| r[0] * s[0] + r[1] * s[1] + r[2] * s[2] + r[3] * s[3] + r[4] * u);
        next[0] /= 1.0 + next[0] * next[0];
        self.state = next;
        next[3]
    }
}

/// The [`RATE_BEATS`] index of LFO Rate (p20) or Phaser Speed (p39): trunc(11·v).
fn rate_index(v: f32) -> usize {
    ((11.0 * v) as usize).min(11)
}

/// The LFO depth s·a², a = 2·v21 − 1 + `wheel` (the MW term when MW Dest is LFO), s = sign(2·v21 − 1).
/// sign(0) = +1, so the wheel still opens a centred LFO Amnt.
fn lfo_depth(amount: f32, wheel: f32) -> f32 {
    let centred = 2.0 * amount - 1.0;
    let a = centred + wheel;
    if centred < 0.0 { -a * a } else { a * a }
}

/// u = 0.95v + 0.05, the envelope time variable.
fn env_u(v: f32) -> f32 {
    0.95f32.mul_add(v, 0.05)
}

/// Attack (p13, p22): k = 1 below 0.001, else 4·10⁻⁷/u⁴.
fn attack_rate(v: f32) -> f32 {
    if v < 0.001 {
        1.0
    } else {
        4e-7 / env_u(v).powi(4)
    }
}

/// Decay (p14, p23) and the filter release (p16): k = 1 below 0.0177, else 4·10⁻⁶/u⁴.
fn decay_rate(v: f32) -> f32 {
    if v < 0.0177 {
        1.0
    } else {
        4e-6 / env_u(v).powi(4)
    }
}

/// Amp release (p25): k = 4·10⁻⁵/u'⁴, u' = 0.95u + 0.05.
fn amp_release_rate(v: f32) -> f32 {
    4e-5 / env_u(env_u(v)).powi(4)
}

/// Glide (Slide p3), per control block: p += c·(key − p), c = 0.9·(1 − v)⁴ + 0.01.
fn glide_rate(v: f32) -> f32 {
    0.9 * (1.0 - v).powi(4) + 0.01
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
enum Stage {
    #[default]
    Off,
    Attack,
    Decay,
    Release,
}

/// e += (target − e)·k per sample; each stage's target and k are latched when it starts. The attack aims at 4 and
/// ends when e passes 1; there e := 1 and the decay aims at the sustain level.
#[derive(Clone, Copy, Default)]
struct Env {
    level: f32,
    target: f32,
    rate: f32,
    stage: Stage,
}

impl Env {
    /// From the current level.
    fn attack(&mut self, rate: f32) {
        (self.stage, self.target, self.rate) = (Stage::Attack, 4.0, rate);
    }

    fn release(&mut self, rate: f32) {
        (self.stage, self.target, self.rate) = (Stage::Release, 0.0, rate);
    }

    /// One sample; `decay` and `sustain` are latched if the attack ends here.
    fn tick(&mut self, decay: f32, sustain: f32) -> f32 {
        self.level += (self.target - self.level) * self.rate;
        if self.stage == Stage::Attack && self.level > 1.0 {
            self.level = 1.0;
            (self.stage, self.target, self.rate) = (Stage::Decay, sustain, decay);
        }
        self.level
    }
}

/// Xorshift32, seeded alike in every instance so renders repeat.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    const SEED: u32 = 0x1234_5678;

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// Uniform in [−1, 1).
    fn uniform(&mut self) -> f32 {
        (self.next() >> 8) as f32 * (1.0 / 8_388_608.0) - 1.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Poly,
    Mono,
    Lead,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    /// Sounding or releasing. A free voice renders nothing and takes the next POLY note first.
    busy: bool,
    key: u8,
    velocity: f32,
    pan: f32,
    /// Note-on serial: POLY steals the oldest-started voice.
    started: u64,
    /// Piano-roll slide offsets: pitch in semitones, pan, velocity.
    slide: [f32; 3],
    amp: Env,
    filter_env: Env,
    /// The glided key, in semitones.
    pitch: f32,
    drift: f32,
    /// The block's base frequency f in Hz.
    frequency: f32,
    /// The table every oscillator of the voice reads this block.
    wave: &'static [f32],
    /// 32-bit oscillator phases, randomized when the voice frees, and the block's increments (0 = unused).
    phases: [u32; 8],
    increments: [u32; 8],
    /// The block's fixed phase offsets from oscillator 1 (or 2), per kind.
    offsets: [u32; 5],
    /// Smoothed L and R gains, and their targets: gain × the pan law.
    gain: [f32; 2],
    target: [f32; 2],
    /// The block's cutoff fc in Hz, and the ladder set to it.
    cutoff: f32,
    ladder: Ladder,
    /// The DC blocker's low-pass state.
    dc: f32,
}

impl Voice {
    /// One sample of OSC Kind `kind`: S(x) reads the voice's table at the top 11 bits of x + the
    /// sample's `dither`. Same-frequency partners read oscillator 1 (2) plus a fixed offset; then every oscillator
    /// advances.
    fn oscillators(&mut self, kind: usize, dither: u32) -> f32 {
        let wave = self.wave;
        let s = |phase: u32| wave[(phase.wrapping_add(dither) >> 21) as usize];
        let [a, b, c, ..] = self.phases;
        let d = self.offsets[0];
        let out = match kind {
            0 => 2.0 * s(a) - s(a.wrapping_add(d)),
            1 => s(a) + s(b),
            2 => s(a) - s(a.wrapping_add(d)),
            3 => s(a) - s(a.wrapping_add(d)) + s(b),
            4 => s(a) - s(a.wrapping_add(d)) + s(b) - s(b.wrapping_add(d)),
            5 => 0.5 * self.phases.iter().map(|&phase| s(phase)).sum::<f32>(),
            6 => {
                0.5 * (s(a)
                    + self
                        .offsets
                        .iter()
                        .map(|&k| s(a.wrapping_add(k)))
                        .sum::<f32>())
            }
            7 => s(a) + s(a.wrapping_add(d)),
            _ => s(a) + s(b) + s(c),
        };
        for (phase, step) in self.phases.iter_mut().zip(self.increments) {
            *phase = phase.wrapping_add(step);
        }
        out
    }
}

/// The block-rate values every voice reads.
#[derive(Clone, Copy, Default)]
struct Control {
    /// The decay rates and sustain levels a decay latches when an attack ends in this block: [amp, filter].
    decay: [f32; 2],
    sustain: [f32; 2],
    glide: f32,
    transpose: f32,
    /// OSC Kind trunc(9·v6), the Modifier m (clamped ±1) and its interval r.
    kind: usize,
    modifier: f32,
    interval: f32,
    noise: f32,
    /// LFO → FRQ: 12·L semitones.
    lfo_pitch: f32,
    /// The voice-independent part of log2 f₀: 4 + 10.3·v9 + the LFO and mod-wheel octaves.
    log2_cutoff: f32,
    /// 2^(10·(2v17 − 1)), the cutoff factor at full filter env.
    env_amount: f32,
    /// k = 4.05·0.99·v10.
    resonance: f32,
    /// Distortion: D = 20·v27², the gain 0.7·(1 − D/16)⁸ + 0.3, and the Decimator's hold h = trunc(64·v28²).
    drive: f32,
    drive_gain: f32,
    hold: usize,
    /// Delay: 0.25·(trunc(3·v30) + 1) beats, halved until ≤ 1.5 s, in frames; fb = v31; the loop low-pass
    /// coefficient 0.99·v32 + 0.01; the mix v33².
    delay_frames: usize,
    delay_feedback: f32,
    delay_coefficient: f32,
    delay_mix: f32,
    /// Phaser: s's target max(v43², 10⁻⁵), the sweep width v40³, cos 2πφ (φ from Speed as for the LFO), the
    /// feedback 0.98·v41^(1/3) and the mix max(v42, 10⁻⁵).
    phaser_target: f32,
    phaser_width: f32,
    phaser_cos: f32,
    phaser_feedback: f32,
    phaser_mix: f32,
    /// Chorus: N, d = 0.36·v36 + 0.04, the rates 4·v35² + 0.2 Hz (L) and + 0.25 Hz (R), the mix v37².
    chorus_frames: usize,
    chorus_depth: f32,
    chorus_rates: [f32; 2],
    chorus_mix: f32,
}

#[derive(Clone, Copy, Default)]
struct Allpass {
    previous_input: f32,
    previous_output: f32,
}

impl Allpass {
    fn process(&mut self, input: f32, coefficient: f32) -> f32 {
        let output =
            -coefficient * input + self.previous_input + coefficient * self.previous_output;
        self.previous_input = input;
        self.previous_output = output;
        output
    }
}

pub struct MiniSynth {
    params: [f32; 44],
    sample_rate: f32,
    tables: &'static [f32],
    tempo_bpm: f32,
    /// The song position in beats: the transport's while it plays, else running freely at the tempo.
    beat: f64,
    voices: [Voice; VOICES],
    /// Note-on count, for the voices' start order.
    serial: u64,
    /// The previously pressed key: every note-on glides from it (none after reset).
    last_key: Option<u8>,
    /// Each held key's velocity, or [`NOT_HELD`]: LEAD's legato and note-off fall back on held keys.
    held: [f32; 128],
    /// Channel Pitch Bend, 4w − 2 semitones (fixed ±2, unsmoothed).
    bend: f32,
    mod_wheel: f32,
    rng: Rng,
    /// Frames into the current control block.
    block_pos: usize,
    /// Whether this block renders: a voice is busy, the 0.5 s FX tail after the last one freed is not over, or the
    /// delay still rings above 10⁻⁵. Otherwise the module outputs nothing and the FX states freeze.
    running: bool,
    tail: usize,
    control: Control,
    decimator_count: usize,
    decimator_sample: [f32; 2],
    /// The delay's mono loop buffer (1.5 s), its write position and the loop low-pass.
    delay: Vec<f32>,
    delay_pos: usize,
    delay_lp: f32,
    /// The delay output's peak in the current block.
    delay_peak: f32,
    /// The chorus's mono buffer (N + 2 frames), its write position and the two LFO phases.
    chorus: Vec<f32>,
    chorus_pos: usize,
    chorus_phase: [f32; 2],
    /// The phaser: six allpasses per channel, each channel's last output (fed back), and the swept frequency s.
    phaser_state: [[Allpass; 6]; 2],
    phaser_out: [f32; 2],
    phaser_sweep: f32,
}

impl MiniSynth {
    pub fn new(sample_rate: f32) -> Result<Self, MiniSynthError> {
        if !sample_rate.is_finite() || !(1.0..=384_000.0).contains(&sample_rate) {
            return Err(MiniSynthError::InvalidSampleRate);
        }
        let mut synth = Self {
            params: std::array::from_fn(|index| PARAMETERS[index].default),
            sample_rate,
            tables: tables(),
            tempo_bpm: 120.0,
            beat: 0.0,
            voices: [Voice::default(); VOICES],
            serial: 0,
            last_key: None,
            held: [NOT_HELD; 128],
            bend: 0.0,
            mod_wheel: 0.0,
            rng: Rng(Rng::SEED),
            block_pos: 0,
            running: false,
            tail: 0,
            control: Control::default(),
            decimator_count: 0,
            decimator_sample: [0.0; 2],
            delay: vec![0.0; (sample_rate * 1.5).ceil() as usize + 2],
            delay_pos: 0,
            delay_lp: 0.0,
            delay_peak: 0.0,
            chorus: vec![0.0; chorus_frames(sample_rate) + 2],
            chorus_pos: 0,
            chorus_phase: CHORUS_PHASES,
            phaser_state: [[Allpass::default(); 6]; 2],
            phaser_out: [0.0; 2],
            phaser_sweep: 0.0,
        };
        synth.reset();
        Ok(synth)
    }

    pub fn parameter(&self, index: usize) -> Option<f32> {
        self.params.get(index).copied()
    }

    /// Enable (p0) is on at v ≥ 0.5; its On→Off edge resets the synth at once. The FX toggles are on at v ≠ 0.
    pub fn set_parameter(&mut self, index: usize, value: f32) -> Result<(), MiniSynthError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(MiniSynthError::InvalidValue);
        }
        let param = self
            .params
            .get_mut(index)
            .ok_or(MiniSynthError::InvalidParameter(index))?;
        let switched_off = index == 0 && *param >= 0.5 && value < 0.5;
        let delay_toggled = index == 29 && (*param != 0.0) != (value != 0.0);
        let chorus_on = index == 34 && *param == 0.0 && value != 0.0;
        *param = value;
        if switched_off {
            self.reset();
        }
        // The delay's buffer is cleared when its toggle changes.
        if delay_toggled {
            self.delay.fill(0.0);
            self.delay_lp = 0.0;
        }
        // The chorus phases restart when it is switched on.
        if chorus_on {
            self.chorus_phase = CHORUS_PHASES;
        }
        Ok(())
    }

    pub fn set_tempo_bpm(&mut self, tempo_bpm: f32) -> Result<(), MiniSynthError> {
        if !tempo_bpm.is_finite() || tempo_bpm <= 0.0 {
            return Err(MiniSynthError::InvalidValue);
        }
        self.tempo_bpm = tempo_bpm;
        Ok(())
    }

    pub fn set_mod_wheel(&mut self, value: f32) -> Result<(), MiniSynthError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(MiniSynthError::InvalidValue);
        }
        self.mod_wheel = value;
        Ok(())
    }

    /// The song position of the next frame while the transport plays: the LFO and the phaser lock to it. When it is
    /// not called the position runs on at the tempo.
    pub fn set_song_beat(&mut self, beat: f64) {
        if beat.is_finite() {
            self.beat = beat;
        }
    }

    /// Channel Pitch Bend v (channel param 4, 0..=1): 4v − 2 semitones on every voice from the next control block.
    pub fn set_pitch_bend(&mut self, v: f32) {
        if v.is_finite() {
            self.bend = 4.0 * v.clamp(0.0, 1.0) - 2.0;
        }
    }

    /// A piano-roll slide event: the offsets of every voice, pitch in semitones, pan and velocity.
    pub fn slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        if pitch.is_finite() && pan.is_finite() && velocity.is_finite() {
            for voice in &mut self.voices {
                voice.slide = [pitch, pan, velocity];
            }
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.note_on_with_pan(note, velocity, 0.5);
    }

    fn enabled(&self) -> bool {
        self.params[0] >= 0.5
    }

    /// Voices Mode (p1): trunc(2v), as the module's other menus.
    fn mode(&self) -> Mode {
        match (2.0 * self.params[1]) as usize {
            0 => Mode::Poly,
            1 => Mode::Mono,
            _ => Mode::Lead,
        }
    }

    /// Ignored while Enable is off. POLY takes the highest-numbered free voice, else steals the oldest-started; MONO
    /// and LEAD use voice 1. LEAD is legato while another key is held and the voice is not released.
    pub fn note_on_with_pan(&mut self, note: u8, velocity: f32, pan: f32) {
        if note > 127
            || !velocity.is_finite()
            || velocity <= 0.0
            || !pan.is_finite()
            || !self.enabled()
        {
            return;
        }
        let key = usize::from(note);
        let others_held = self
            .held
            .iter()
            .enumerate()
            .any(|(held, &v)| held != key && v != NOT_HELD);
        self.held[key] = velocity;
        let mode = self.mode();
        let start = match mode {
            Mode::Lead if !others_held => note,
            _ => self.last_key.unwrap_or(note),
        };
        self.last_key = Some(note);
        let index = match mode {
            Mode::Poly => (0..VOICES)
                .rev()
                .find(|&i| !self.voices[i].busy)
                .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].started).unwrap()),
            Mode::Mono | Mode::Lead => 0,
        };
        self.serial += 1;
        let (attack, filter_attack) = (attack_rate(self.params[22]), attack_rate(self.params[13]));
        let voice = &mut self.voices[index];
        if mode == Mode::Lead && others_held && voice.busy && voice.amp.stage != Stage::Release {
            (voice.key, voice.velocity) = (note, velocity);
            return;
        }
        let was_free = !voice.busy;
        if was_free {
            *voice = Voice {
                drift: voice.drift,
                phases: voice.phases,
                ..Voice::default()
            };
        }
        voice.busy = true;
        (voice.key, voice.velocity, voice.pan) = (note, velocity, pan);
        voice.started = self.serial;
        voice.pitch = f32::from(start);
        voice.amp.attack(attack);
        voice.filter_env.level = 0.0;
        voice.filter_env.attack(filter_attack);
        self.update_control();
        self.voice_control(index);
        self.running = true;
        self.tail = self.tail_frames();
        if was_free {
            let voice = &mut self.voices[index];
            voice.gain = voice.target;
        }
    }

    /// Ignored while Enable is off. POLY and MONO release every voice on the key. LEAD, when the key is the sounding
    /// one and other keys are held, moves the voice to the highest held key below, else the lowest above, with that
    /// key's velocity and no retrigger.
    pub fn note_off(&mut self, note: u8) {
        if note > 127 || !self.enabled() {
            return;
        }
        self.held[usize::from(note)] = NOT_HELD;
        let mode = self.mode();
        let lead = &mut self.voices[0];
        if mode == Mode::Lead && lead.busy && lead.key == note && lead.amp.stage != Stage::Release {
            let held = |key: &usize| self.held[*key] != NOT_HELD;
            let below = (0..usize::from(note)).rev().find(held);
            if let Some(key) = below.or_else(|| (usize::from(note) + 1..128).find(held)) {
                (lead.key, lead.velocity) = (key as u8, self.held[key]);
                return;
            }
        }
        let (release, filter_release) = (
            amp_release_rate(self.params[25]),
            decay_rate(self.params[16]),
        );
        for voice in &mut self.voices {
            if voice.busy && voice.key == note && voice.amp.stage != Stage::Release {
                voice.amp.release(release);
                voice.filter_env.release(filter_release);
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        self.held = [NOT_HELD; 128];
        let (release, filter_release) = (
            amp_release_rate(self.params[25]),
            decay_rate(self.params[16]),
        );
        for voice in &mut self.voices {
            if voice.busy && voice.amp.stage != Stage::Release {
                voice.amp.release(release);
                voice.filter_env.release(filter_release);
            }
        }
    }

    /// Silences at once, as a new instance: voices, glide memory, held keys, effect tails and phases; params,
    /// tempo, bend and the mod wheel are kept.
    pub fn reset(&mut self) {
        self.rng = Rng(Rng::SEED);
        self.voices = [Voice::default(); VOICES];
        for voice in &mut self.voices {
            voice.phases = std::array::from_fn(|_| self.rng.next());
        }
        self.serial = 0;
        self.last_key = None;
        self.held = [NOT_HELD; 128];
        self.block_pos = 0;
        self.running = false;
        self.tail = 0;
        self.delay_peak = 0.0;
        self.beat = 0.0;
        self.decimator_count = 0;
        self.decimator_sample = [0.0; 2];
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.delay_lp = 0.0;
        self.chorus.fill(0.0);
        self.chorus_pos = 0;
        self.chorus_phase = CHORUS_PHASES;
        self.phaser_state = [[Allpass::default(); 6]; 2];
        self.phaser_out = [0.0; 2];
        self.update_control();
        // s starts at its target.
        self.phaser_sweep = self.control.phaser_target;
    }

    /// Adds one block into `left` and `right`. Nothing renders while Enable is off.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), MiniSynthError> {
        if left.len() != right.len() {
            return Err(MiniSynthError::MismatchedBuffers);
        }
        if !self.enabled() {
            return Ok(());
        }
        let beats_per_frame = f64::from(self.tempo_bpm) / (60.0 * f64::from(self.sample_rate));
        let mut done = 0;
        while done < left.len() {
            if self.block_pos == 0 {
                self.control_block();
            }
            let frames = (BLOCK - self.block_pos).min(left.len() - done);
            let rendered = if self.running { frames } else { 0 };
            for (l, r) in left[done..done + rendered]
                .iter_mut()
                .zip(&mut right[done..done + rendered])
            {
                let sample = self.voices_frame();
                let [wet_l, wet_r] = self.effects(sample);
                *l += wet_l;
                *r += wet_r;
            }
            self.block_pos = (self.block_pos + frames) % BLOCK;
            self.beat += frames as f64 * beats_per_frame;
            done += frames;
        }
        Ok(())
    }

    /// The block-rate laws: the shared values, then each sounding voice's subnormal flush, glide, drift and pitch.
    fn control_block(&mut self) {
        let busy = self.voices.iter().any(|voice| voice.busy);
        self.tail = if busy {
            self.tail_frames()
        } else {
            self.tail.saturating_sub(BLOCK)
        };
        let echoing = self.params[29] != 0.0 && self.delay_peak > 1e-5;
        self.running = busy || self.tail > 0 || echoing;
        self.delay_peak = 0.0;
        self.update_control();
        for index in 0..VOICES {
            let voice = &mut self.voices[index];
            if voice.busy {
                let states = [
                    &mut voice.amp.level,
                    &mut voice.filter_env.level,
                    &mut voice.dc,
                ];
                for x in states.into_iter().chain(&mut voice.ladder.state) {
                    if x.abs() < FLUSH {
                        *x = 0.0;
                    }
                }
                voice.pitch += self.control.glide * (f32::from(voice.key) - voice.pitch);
                voice.drift += 0.01 * (self.rng.uniform() - voice.drift);
                self.voice_control(index);
            }
        }
    }

    /// The FX tail after the last voice frees: 0.5 s.
    fn tail_frames(&self) -> usize {
        (0.5 * self.sample_rate) as usize
    }

    /// The shared block values, from the params as they are now: at each block start, and at a note-on so the new
    /// voice starts from the current params.
    fn update_control(&mut self) {
        let p = &self.params;
        // The LFO: φ = frac(beats / period); L = depth × the shape's wave.
        let phase = (self.beat / RATE_BEATS[rate_index(p[20])]).rem_euclid(1.0) as f32;
        let wave = match ((2.0 * p[18]) as usize).min(2) {
            0 => (TAU * phase).sin(),
            1 => 1.0 - 2.0 * phase,
            _ => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        };
        // Destinations: LFO Dest trunc(2·v19) FLT / FRQ / OSCMOD, MW Dest trunc(2·v4) FLT / LFO / OSCMOD.
        let wheel = p[5] * self.mod_wheel;
        let wheel_to = |dest: usize| {
            if ((2.0 * p[4]) as usize).min(2) == dest {
                wheel
            } else {
                0.0
            }
        };
        let lfo = lfo_depth(p[21], wheel_to(1)) * wave;
        let lfo_dest = ((2.0 * p[19]) as usize).min(2);
        let lfo_to = |dest: usize| if lfo_dest == dest { lfo } else { 0.0 };
        let modifier = (p[7] + wheel_to(2) + lfo_to(2)).clamp(-1.0, 1.0);
        let drive = 20.0 * p[27] * p[27];
        let mut seconds =
            0.25 * (((3.0 * p[30]) as usize).min(3) + 1) as f32 * 60.0 / self.tempo_bpm;
        while seconds > 1.5 {
            seconds /= 2.0;
        }
        self.control = Control {
            decay: [decay_rate(p[23]), decay_rate(p[14])],
            sustain: [p[24], p[15]],
            glide: glide_rate(p[3]),
            transpose: transpose_steps(p[2]) as f32,
            kind: ((9.0 * p[6]) as usize).min(9),
            modifier,
            interval: interval(modifier),
            noise: p[8] * p[8],
            lfo_pitch: 12.0 * lfo_to(1),
            log2_cutoff: 4.0 + 10.3 * p[9] + 4.0 * lfo_to(0) + 4.0 * wheel_to(0),
            env_amount: (10.0 * (2.0 * p[17] - 1.0)).exp2(),
            resonance: 4.05 * 0.99 * p[10],
            drive,
            drive_gain: 0.7 * (1.0 - drive / 16.0).powi(8) + 0.3,
            hold: (64.0 * p[28] * p[28]) as usize,
            delay_frames: ((seconds * self.sample_rate) as usize).clamp(1, self.delay.len() - 1),
            delay_feedback: p[31],
            delay_coefficient: 0.99 * p[32] + 0.01,
            delay_mix: p[33] * p[33],
            phaser_target: (p[43] * p[43]).max(1e-5),
            phaser_width: p[40] * p[40] * p[40],
            phaser_cos: (TAU * (self.beat / RATE_BEATS[rate_index(p[39])]).rem_euclid(1.0) as f32)
                .cos(),
            phaser_feedback: 0.98 * p[41].cbrt(),
            phaser_mix: p[42].max(1e-5),
            chorus_frames: chorus_frames(self.sample_rate),
            chorus_depth: 0.36 * p[36] + 0.04,
            chorus_rates: [0.2, 0.25].map(|hz| 4.0 * p[35] * p[35] + hz),
            chorus_mix: p[37] * p[37],
        };
    }

    /// One voice's block values: P = glided key + Transpose + bend + LFO + slide pitch + 0.08·drift semitones; the gain
    /// targets clamp(velocity + slide velocity, 0, 1) × (L = min(1, 2 − 2p), R = min(1, 2p)), p = clamp(pan + slide
    /// pan, 0, 1).
    fn voice_control(&mut self, index: usize) {
        let voice = &mut self.voices[index];
        let pitch = voice.pitch
            + self.control.transpose
            + self.bend
            + self.control.lfo_pitch
            + voice.slide[0]
            + 0.08 * voice.drift;
        let f = 440.0 * ((pitch - 69.0) / 12.0).exp2();
        voice.frequency = f;
        let (kind, m) = (self.control.kind, self.control.modifier);
        let ratios: [f32; 8] = match kind {
            1 | 3 | 4 => [1.0, self.control.interval, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            5 => SUPERSAW_SPREAD.map(|c| 1.0 + m * c),
            8 => [
                0.5 - 0.005 * m,
                1.0,
                2.0 + 0.02 * m,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            9 => [
                1.5 - 0.015 * m,
                1.0,
                2.0 + 0.02 * m,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
            _ => [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        };
        let sr = self.sample_rate;
        let h = harmonics(f * ratios[0], sr);
        voice.wave = &self.tables[(h - 1) * TABLE..h * TABLE];
        voice.increments = ratios.map(|ratio| increment(f * ratio, sr));
        // log2 f₀ = 4 + 10.3·v9 + v11·(P/12 − 4) + 14·v12·(vel − 0.5) + LFO + wheel + 0.103·drift; the filter env F
        // at the block start moves fc from f₀ toward f₀·2^(10·(2v17 − 1)), linearly in Hz; fc ≤ 20 kHz.
        let c = self.control;
        let f0 = (c.log2_cutoff
            + self.params[11] * (pitch / 12.0 - 4.0)
            + 14.0 * self.params[12] * (voice.velocity - 0.5)
            + 0.103 * voice.drift)
            .exp2();
        voice.cutoff =
            (f0 + (f0 * c.env_amount - f0) * voice.filter_env.level).clamp(0.0, 20_000.0);
        let g = (f64::from(voice.cutoff) * 6.25 / (4096.0 * f64::from(sr))).clamp(0.0, 1.0);
        voice.ladder.set(g, f64::from(c.resonance));
        voice.offsets = match kind {
            0 | 2 | 7 => [cycles(1.0 - m.abs()), 0, 0, 0, 0],
            3 => [1 << 31, 0, 0, 0, 0],
            4 => [cycles(0.5625), 0, 0, 0, 0],
            6 => std::array::from_fn(|k| cycles((k + 1) as f32 * m.abs() / 2.0)),
            _ => [0; 5],
        };
        let gain = (voice.velocity + voice.slide[2]).clamp(0.0, 1.0);
        let pan = (voice.pan + voice.slide[1]).clamp(0.0, 1.0);
        voice.target = [
            gain * (2.0 - 2.0 * pan).min(1.0),
            gain * (2.0 * pan).min(1.0),
        ];
    }

    /// One frame of the voice sum. Per voice: u = ((osc + noise)·0.2 + 0.07)·A → filter → ×5 → DC blocker
    /// (one-pole high-pass, 0.005) → × the smoothed gains (0.005 per sample). In release the gain targets drop to 0
    /// once A < 10⁻⁵, and the voice frees when both gains are below 10⁻⁵.
    fn voices_frame(&mut self) -> [f32; 2] {
        let c = self.control;
        let mut sum = [0.0; 2];
        for voice in &mut self.voices {
            if !voice.busy {
                continue;
            }
            let amp = voice.amp.tick(c.decay[0], c.sustain[0]);
            voice.filter_env.tick(c.decay[1], c.sustain[1]);
            let dither = self.rng.next() >> 10;
            let mut signal = voice.oscillators(c.kind, dither);
            if c.noise > 0.0 {
                signal += self.rng.uniform() * c.noise;
            }
            let u = (signal * 0.2 + 0.07) * amp;
            let y = voice.ladder.tick(u) * 5.0;
            let y = y - voice.dc;
            voice.dc += 0.005 * y;
            let released = voice.amp.stage == Stage::Release;
            let target = if released && amp < 1e-5 {
                [0.0; 2]
            } else {
                voice.target
            };
            for (channel, (gain, target)) in voice.gain.iter_mut().zip(target).enumerate() {
                *gain += (target - *gain) * 0.005;
                sum[channel] += y * *gain;
            }
            if released && voice.gain[0] < 1e-5 && voice.gain[1] < 1e-5 {
                voice.busy = false;
                voice.phases = std::array::from_fn(|_| self.rng.next());
            }
        }
        sum
    }

    /// The built-in FX after the voice sum, in this order: Distortion → Delay → Phaser → Chorus, each while its
    /// toggle is not 0.
    fn effects(&mut self, mut x: [f32; 2]) -> [f32; 2] {
        if self.params[26] != 0.0 {
            x = self.distortion(x);
        }
        if self.params[29] != 0.0 {
            x = self.delay(x);
        }
        if self.params[38] != 0.0 {
            x = self.phaser(x);
        }
        if self.params[34] != 0.0 {
            x = self.chorus(x);
        }
        x
    }

    /// D = 20·v27²: y = 1 − (1 − x)·10^(−D·x) for x ≥ 0, −1 + (1 + x)·10^(D·x) below, × (0.7·(1 − D/16)⁸ + 0.3). Then
    /// the Decimator holds every h-th frame, h = trunc(64·v28²) (h ≤ 1: off).
    fn distortion(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &self.control;
        let mut y = x.map(|x| overdrive(x, c.drive) * c.drive_gain);
        if c.hold > 1 {
            if self.decimator_count == 0 {
                self.decimator_sample = y;
            }
            y = self.decimator_sample;
            self.decimator_count = (self.decimator_count + 1) % c.hold;
        }
        y
    }

    /// Mono in: y = LP(fb·buffer out) with the one-pole coefficient 0.99·v32 + 0.01, the buffer takes L + R + y, and
    /// y·v33² is added to both channels; the dry signal is unchanged.
    fn delay(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &self.control;
        let len = self.delay.len();
        let out = self.delay[(self.delay_pos + len - c.delay_frames) % len];
        self.delay_lp += (c.delay_feedback * out - self.delay_lp) * c.delay_coefficient;
        let y = self.delay_lp;
        self.delay[self.delay_pos] = x[0] + x[1] + y;
        self.delay_pos = (self.delay_pos + 1) % len;
        self.delay_peak = self.delay_peak.max(y.abs());
        x.map(|x| x + y * c.delay_mix)
    }

    /// Mono in (L + R) to a buffer of N frames; m = 0.5 + d·sin 2πφ per side. L reads the buffer N − m²·d·N frames
    /// back with linear interpolation; R crossfades (1 − m)·x[n] + m·x[n − N + 1]. Wet × v37² is added to the dry signal.
    fn chorus(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &self.control;
        let (len, n) = (self.chorus.len(), c.chorus_frames);
        self.chorus[self.chorus_pos] = x[0] + x[1];
        let at = |back: usize| self.chorus[(self.chorus_pos + len - back) % len];
        let [m_l, m_r] = self
            .chorus_phase
            .map(|phase| 0.5 + c.chorus_depth * (TAU * phase).sin());
        let delay = n as f32 * (1.0 - m_l * m_l * c.chorus_depth);
        let back = delay as usize;
        let wet_l = at(back) + (at(back + 1) - at(back)) * delay.fract();
        let wet_r = (1.0 - m_r) * at(0) + m_r * at(n - 1);
        for (phase, hz) in self.chorus_phase.iter_mut().zip(c.chorus_rates) {
            *phase = (*phase + hz / self.sample_rate).fract();
        }
        self.chorus_pos = (self.chorus_pos + 1) % len;
        [x[0] + wet_l * c.chorus_mix, x[1] + wet_r * c.chorus_mix]
    }

    /// Six first-order allpasses per channel with a = (1 − x)/(1 + x), x = (50 + 19950·(s + lfo))/sr clamped to
    /// [10⁻⁵, 0.499]; s moves toward max(v43², 10⁻⁵) by 0.001 per sample; lfo_L,R = (1 ± cos 2πφ)·v40³/2. Stage 6
    /// feeds back 0.98·v41^(1/3) to the input; the wet signal × max(v42, 10⁻⁵) is added to the dry one.
    fn phaser(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &self.control;
        self.phaser_sweep += (c.phaser_target - self.phaser_sweep) * 0.001;
        let lfo = [1.0 + c.phaser_cos, 1.0 - c.phaser_cos].map(|lfo| lfo * c.phaser_width / 2.0);
        let mut out = x;
        for (channel, lfo) in lfo.into_iter().enumerate() {
            let f = ((50.0 + 19_950.0 * (self.phaser_sweep + lfo)) / self.sample_rate)
                .clamp(1e-5, 0.499);
            let a = (1.0 - f) / (1.0 + f);
            let mut wet = x[channel] + c.phaser_feedback * self.phaser_out[channel];
            for stage in &mut self.phaser_state[channel] {
                wet = stage.process(wet, a);
            }
            self.phaser_out[channel] = wet;
            out[channel] += wet * c.phaser_mix;
        }
        out
    }
}

/// The chorus buffer N = trunc(0.01·sr) frames (at least 2).
fn chorus_frames(sample_rate: f32) -> usize {
    ((0.01 * sample_rate) as usize).max(2)
}

/// The Overdrive curve at drive D.
fn overdrive(x: f32, drive: f32) -> f32 {
    if x >= 0.0 {
        1.0 - (1.0 - x) * 10f32.powf(-drive * x)
    } else {
        -1.0 + (1.0 + x) * 10f32.powf(drive * x)
    }
}

#[cfg(test)]
mod tests;
