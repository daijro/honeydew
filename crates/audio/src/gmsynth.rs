//! GMSynth (instrument ID 2), term by term. Two additive-wavetable
//! oscillators with choir copies and osc mod, noise, an 8-stage one-pole filter, four one-pole envelopes, one global
//! LFO, mod wheel and pitch bend, master level and pan. vₙ below is param n. Rates are per sample unless marked
//! "block" (32 samples); the block grid runs on across `process` calls, so any block split renders alike.
//!
//! The summed voice bus runs FX 1 → FX 2 → FX 3 (p46–p66), each a MultiFX processor bank with DIST and RING
//! ([`crate::multifx`]), then the master level and pan.
//!
//! Tables are built on the app thread and shared:
//! - **Shapes.** A process-wide cache holds one `Arc<Wavetable>` per menu slot, built on first use by [`GmSynth::new`]
//!   (the default shape) and [`prepare_setting`]. [`Instrument::set_setting`] runs on the audio thread and only takes
//!   an `Arc` clone of a built slot: an atomic increment, no allocation or lock. The cache keeps its own reference, so
//!   dropping a voice's old shape there never frees it. A slot not yet built reads as a missing shape (silent).
//! - **The filter table** is built once per process, at the first construction.

use std::f32::consts::TAU;
use std::f64::consts::{PI, TAU as TAU_64};
use std::sync::{Arc, OnceLock};

use crate::ProcessContext;
use crate::gm_waveshapes::{SLOTS, Spectrum, Wavetable, read, spectrum, table_index_for};
use crate::instrument::Instrument;
use crate::multifx::{Bank, Settings};
use crate::voice::{GMSYNTH_RATE_BEATS, tempo_step};

/// Params 0–74.
pub const PARAMS: usize = 75;
/// Voices.
pub const VOICES: usize = 4;
/// Choir copies at most: Solo … Quartet.
const COPIES: usize = 4;
/// The control block: "block" laws update every 32 samples.
const BLOCK: usize = 32;

/// Param defaults. A menu's default is its exact step k/(n − 1): the table's
/// 0.1111 is 1/9 rounded for display.
#[rustfmt::skip]
pub const DEFAULTS: [f32; PARAMS] = [
    1.0, 0.7273, 0.5, 0.0, 1.0, 1.0, 0.2, 0.5, 0.5, 0.5, 0.5, // 0–10
    0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, // 11–20
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.0, 0.0, // 21–30
    0.0, 1.0, 0.0, 0.5, 0.0, 0.0, 1.0, 1.0, 1.0, 0.5, // 31–40
    0.0, 0.0, 0.0, 0.0, 0.0, 1.0 / 9.0, 0.0, 0.0, 0.0, 0.5, // 41–50
    0.5, 0.5, 5.0 / 9.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 3.0 / 9.0, // 51–60
    0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, // 61–70
    0.0, 0.0, 0.0, 0.0, // 71–74
];
/// Enable is on at v ≥ 0.5; MONO (p30), AUTO (p31) and VEL (p74) at any nonzero v.
const ENABLE: usize = 0;
const SINGLE_VOICE: usize = 30;
/// FX slot n's params start at 46 + 7n: Kind, Enable, Sync, LFO Shape, LFO Amount, X, Y.
const FX_PARAMS: usize = 46;
const FX_SLOTS: usize = 3;

/// `CSTM` settings under the keys the panel uses (`src/panels/cstm.rs` `GMSYNTH`).
/// A menu's setting is its item index: OSC A / B shape 0–28, PB RANGE item, MW DEST item; MW AMOUNT is v.
pub const WAVE_A: u32 = 21100;
pub const WAVE_B: u32 = 21200;
pub const PB_RANGE: u32 = 36100;
pub const MW_AMOUNT: u32 = 37000;
pub const MW_DEST: u32 = 36200;
/// PB RANGE items `1 smt`, `2 smt`, `7 smt`, `Oct`; default item 1.
const PB_RANGES: [f32; 4] = [1.0, 2.0, 7.0, 12.0];
/// MW DEST items; default Osc. Mod.
const MW_FILTER: usize = 0;
const MW_LFO: usize = 1;
const MW_OSC_MOD: usize = 2;
/// Shape at creation: Beast.
const DEFAULT_SHAPE: usize = 0;

/// Channel params as [`Instrument::set_controller`] indexes them: Pitch Bend and MIDI CC #1,
/// the mod wheel.
pub const PITCH_BEND: usize = 4;
pub const MOD_WHEEL: usize = 8;

// Osc mod kinds, int(3·v16).
const RING: u8 = 0;
const FM: u8 = 1;
const SYNC: u8 = 2;
const FILTER: u8 = 3;
/// Ring, FM and Filter mod are inactive at m ≤ 0.001.
const MOD_THRESHOLD: f32 = 0.001;

/// One-pole smoothing per sample: v15 + MW, voice gain, master level and pan (authored at 44.1 kHz).
const SMOOTHING: f32 = 0.01;
/// The cutoff base S follows its target at 0.001 per sample.
const CUTOFF_SMOOTHING: f32 = 0.001;
/// KBD tracking per key around key 64: 1.0008 oct/oct.
const KBD_PER_KEY: f32 = 0.008_996_4;
const KBD_CENTRE: f32 = 64.0;
/// Attack aims here and hands over to decay once above 1.
const ATTACK_TARGET: f32 = 5.0;
/// The envelope rate numerator: k = 8.82/(sr·…).
const RATE_SCALE: f32 = 8.82;
/// Envelopes below this are 0, so a long decay to 0 never runs on subnormals (ours, far below hearing).
const FLUSH: f32 = 1e-20;
/// A voice whose amp env aims at 0 goes idle below this level.
const FREE_LEVEL: f32 = 1e-4;
/// The master level is 0 below −60 dB.
const SILENT_DB: f32 = -60.0;

/// Nominal cutoff f = 32·2^(9.27c) Hz.
const CUTOFF_HZ: f32 = 32.0;
const CUTOFF_OCTAVES: f32 = 9.27;
/// The rate the filter table is authored for; elsewhere its frequencies scale with sr/44100.
const TABLE_RATE: f64 = 44_100.0;
/// The table entry for f peaks at f_r ≈ 1.0159·f + 0.5 Hz (a fit of the shipped table, residual < 1 Hz).
const PEAK_SCALE: f32 = 1.0159;
const PEAK_OFFSET_HZ: f32 = 0.5;
/// The regenerated table's axis: entry j peaks at f_r = 11.68 + 0.304433·j Hz (a fit, not an exact constant), read
/// with linear interpolation between entries.
const AXIS_START_HZ: f32 = 11.68;
const AXIS_STEP_HZ: f32 = 0.304_433;
/// Entries (65536 puts the last at 19.96 kHz, where b = 9.86, the top; entry 0 gives the low 1.888).
const FILTER_TABLE_LEN: usize = 65_536;
/// One-pole stages per channel.
const STAGES: usize = 8;
/// Soft clip x/(1 + 0.05|x|) at the filter input.
const CLIP: f32 = 0.05;

/// The xorshift seed of the dither and phase generator, and the base seed of each voice's noise (our choice).
const SEED: u32 = 0x2545_F491;

/// The 29 shapes' tables, each built once per process and shared by every instance ("Shared by all instances").
static SHAPES: [OnceLock<Arc<Wavetable>>; SLOTS] = [const { OnceLock::new() }; SLOTS];
static FILTER_TABLE: OnceLock<FilterTable> = OnceLock::new();

/// App thread: builds what a `CSTM` setting needs before [`Instrument::set_setting`] receives it on the audio thread
/// (the OSC A / B shape). The engine calls it at build; a live setting edit calls it before sending.
pub fn prepare_setting(key: u32, value: f32) {
    if matches!(key, WAVE_A | WAVE_B) {
        let _ = shape(value as usize);
    }
}

/// The user's original waveshapes, by slot, set at start ([`install_shape`]).
static ORIGINAL: [OnceLock<Spectrum>; SLOTS] = [const { OnceLock::new() }; SLOTS];

/// App thread, at start, before any GMSynth is built: menu slot `slot` plays `spectrum` (the user's own content) in
/// place of its designed shape. Returns whether it took it: once per slot and process, and never
/// after the slot's tables were built.
pub fn install_shape(slot: usize, spectrum: Spectrum) -> bool {
    let built = SHAPES.get(slot).is_none_or(|cell| cell.get().is_some());
    !built && ORIGINAL[slot].set(spectrum).is_ok()
}

/// App thread: menu slot `slot`'s tables, built on first use (2 MB, a few ms); `None` past the menu.
fn shape(slot: usize) -> Option<Arc<Wavetable>> {
    let cell = SHAPES.get(slot)?;
    let built = cell.get_or_init(|| {
        let original = ORIGINAL[slot].get().cloned();
        Arc::new(Wavetable::build(
            &original.or_else(|| spectrum(slot)).expect("a menu slot"),
        ))
    });
    Some(Arc::clone(built))
}

/// Any thread: `slot`'s tables if they are built. Never builds, allocates or blocks.
fn cached(slot: usize) -> Option<Arc<Wavetable>> {
    SHAPES.get(slot)?.get().cloned()
}

/// The stage coefficient a and the self-oscillation gain b by f_r on the table axis.
struct FilterTable(Box<[[f32; 2]]>);

impl FilterTable {
    /// Entry j: a makes the 8-stage loop, with its 1-sample feedback delay, reach phase −π at ω = 2π·f_r/44100,
    /// i.e. 8·arg H − ω = −π for H = a/(1 − (1 − a)e^{−jω}); b = |H|^−8. With r = 1 − a and
    /// θ = (π − ω)/8, arg H = −atan2(r·sin ω, 1 − r·cos ω) = −θ solves to r = sin θ / sin(ω + θ).
    fn build() -> Self {
        Self(
            (0..FILTER_TABLE_LEN)
                .map(|j| {
                    let f_r = f64::from(AXIS_START_HZ) + f64::from(AXIS_STEP_HZ) * j as f64;
                    let w = TAU_64 * f_r / TABLE_RATE;
                    let theta = (PI - w) / 8.0;
                    let r = theta.sin() / (w + theta).sin();
                    let h = (1.0 - r) / (1.0 - 2.0 * r * w.cos() + r * r).sqrt();
                    [(1.0 - r) as f32, h.powi(-8) as f32]
                })
                .collect(),
        )
    }

    /// (a, b) for normalized cutoff c: nominal f = 32·2^(9.27c), its f_r on the axis, interpolated.
    fn at(&self, c: f32) -> (f32, f32) {
        let f = CUTOFF_HZ * (CUTOFF_OCTAVES * c).exp2();
        let x = ((PEAK_SCALE * f + PEAK_OFFSET_HZ - AXIS_START_HZ) / AXIS_STEP_HZ)
            .clamp(0.0, (FILTER_TABLE_LEN - 1) as f32);
        let j = (x as usize).min(FILTER_TABLE_LEN - 2);
        let t = x - j as f32;
        let ([a0, b0], [a1, b1]) = (self.0[j], self.0[j + 1]);
        (a0 + t * (a1 - a0), b0 + t * (b1 - b0))
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Release,
}

/// Per-sample envelope coefficients, refreshed each block.
#[derive(Clone, Copy, Default)]
struct Rates {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// e += (target − e)·k per sample. Attack aims at 5.0 and ends when e > 1 (e := 1); decay aims at the sustain
/// level, release at 0.
#[derive(Clone, Copy, Default)]
struct Envelope {
    e: f32,
    stage: Stage,
}

impl Envelope {
    fn step(&mut self, rates: &Rates) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.e += (ATTACK_TARGET - self.e) * rates.attack;
                if self.e > 1.0 {
                    self.e = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.e += (rates.sustain - self.e) * rates.decay,
            Stage::Release => self.e -= self.e * rates.release,
            Stage::Idle => {}
        }
        if self.e < FLUSH {
            self.e = 0.0;
        }
        self.e
    }

    /// Restarts the attack from 0 (filter env, mod env, EG).
    fn restart(&mut self) {
        *self = Self {
            e: 0.0,
            stage: Stage::Attack,
        };
    }
}

/// Attack k = 1 at v = 0, else 8.82/(sr·100v⁴): 0 → 1 in ≈ 2.53·v⁴ s.
fn attack_rate(v: f32, sample_rate: f32) -> f32 {
    if v <= 0.0 {
        1.0
    } else {
        RATE_SCALE / (sample_rate * 100.0 * v.powi(4))
    }
}

/// Decay k = 8.82/(sr·(200v⁷ + 0.001)): τ = 22.7·v⁷ s + 0.11 ms.
fn decay_rate(v: f32, sample_rate: f32) -> f32 {
    RATE_SCALE / (sample_rate * (200.0 * v.powi(7) + 0.001))
}

/// Everything the block laws give, refreshed every 32 samples.
#[derive(Clone, Copy, Default)]
struct Control {
    amp: Rates,
    filter_env: Rates,
    mod_env: Rates,
    eg: Rates,
    /// Choir N, each copy's frequency ratio and its [L, R] gains.
    copies: usize,
    ratio: [f64; COPIES],
    copy_pan: [[f32; 2]; COPIES],
    gain_a: f32,
    gain_b: f32,
    noise: f32,
    /// f_B/f_A.
    ratio_b: f32,
    /// TRNSP + R·w, in semitones.
    offset: f32,
    slide: f32,
    /// VEL: the Filter and Osc. Mod. EG depths × velocity.
    velocity_depth: bool,
    eg_filter: f32,
    eg_mod: f32,
    eg_pitch: f32,
    lfo_shape: u8,
    lfo_filter: f32,
    lfo_mod: f32,
    /// LFO → FREQ (st) and LFO → MOD at the block start, for the block laws.
    lfo_pitch: f32,
    lfo_mod_now: f32,
    mod_kind: u8,
    /// 2v19 − 1.
    mod_contour: f32,
    /// v15 + MW, the smoother's target; clamp(v15 + MW, 0, 1) for Sync's m′.
    mod_target: f32,
    sync_base: f32,
    /// (2v27 − 1)³.
    filter_contour: f32,
    cutoff: f32,
    kbd: f32,
    mw_filter: f32,
    resonance: f32,
    filter_kind: u8,
    /// Master targets: linear level and pan P.
    level: f32,
    pan: f32,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    key: u8,
    velocity: f32,
    /// The note pan p, and the voice pan [L, R] of clamp(p + slide pan, 0, 1).
    note_pan: f32,
    pan: [f32; 2],
    /// Slide offsets: semitones, pan, velocity; a launched voice starts at 0.
    slide: [f32; 3],
    /// Start serial; the smallest is the oldest.
    age: u64,
    /// The glide pitch p, in keys.
    p: f32,
    amp: Envelope,
    filter_env: Envelope,
    mod_env: Envelope,
    eg: Envelope,
    /// The smoothed velocity gain.
    gain: f32,
    /// Cutoff base S, its block target, and whether S snaps to it (note start).
    s: f32,
    s_target: f32,
    snap: bool,
    phase_a: [u32; COPIES],
    phase_b: [u32; COPIES],
    step_a: [u32; COPIES],
    step_b: [u32; COPIES],
    table_a: usize,
    table_b: usize,
    stages: [[f32; STAGES]; 2],
    noise: u32,
}

impl Voice {
    fn sounding(&self) -> bool {
        self.amp.stage != Stage::Idle
    }

    /// Sounding and not yet released.
    fn held(&self) -> bool {
        matches!(self.amp.stage, Stage::Attack | Stage::Decay)
    }

    /// Note-off: the amp env and EG release; the filter and mod envs ignore it.
    fn release(&mut self) {
        if self.sounding() {
            self.amp.stage = Stage::Release;
            self.eg.stage = Stage::Release;
        }
    }

    /// Glide, once per block: p += (target − p)·n/(0.1·sr·v29 + 1), snapping when it reaches the target.
    fn glide(&mut self, slide: f32, sample_rate: f32) {
        let gap = f32::from(self.key) - self.p;
        let step = gap * BLOCK as f32 / (0.1 * sample_rate * slide + 1.0);
        self.p = if step.abs() >= gap.abs() {
            f32::from(self.key)
        } else {
            self.p + step
        };
    }

    /// The block laws: pitch, both tables, the phase steps and the cutoff target.
    fn control(&mut self, c: &Control, sample_rate: f32) {
        // EG → Pitch and the EG term in Sync's m′ ignore VEL.
        let eg = self.eg.e;
        // Pitch = p + TRNSP + R·w + slide + [EG→Pitch] + [LFO→FREQ]; f_A = 220·2^((pitch − 57)/12).
        let pitch = self.p + c.offset + self.slide[0] + c.eg_pitch * eg + c.lfo_pitch;
        let f_a = 220.0 * ((pitch - 57.0) / 12.0).exp2();
        let f_b = f_a * c.ratio_b;
        self.table_a = table_index_for(f_a, sample_rate);
        self.table_b = table_index_for(f_b, sample_rate);
        // Sync: A runs at f_A·2^m′, m′ = clamp(v15 + MW, 0, 1) + (2v19 − 1)·E_mod + mods, unclamped.
        let sync = if c.mod_kind == SYNC {
            (c.sync_base + c.mod_contour * self.mod_env.e + c.eg_mod * eg + c.lfo_mod_now).exp2()
        } else {
            1.0
        };
        for (k, &ratio) in c.ratio.iter().enumerate() {
            self.step_a[k] = phase_step(f_a * sync, ratio, sample_rate);
            self.step_b[k] = phase_step(f_b, ratio, sample_rate);
        }
        self.s_target =
            (c.cutoff + c.kbd * (pitch - KBD_CENTRE) * KBD_PER_KEY + c.mw_filter).clamp(0.0, 1.0);
        if self.snap {
            self.s = self.s_target;
            self.snap = false;
        }
    }

    /// Adds this voice into `bus` for one stretch of a block: `dither`, `lfo` and `amount` (the smoothed v15 + MW)
    /// hold one value per sample.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        c: &Control,
        (wave_a, wave_b): (&Wavetable, &Wavetable),
        filter: &FilterTable,
        dither: &[u32],
        lfo: &[f32],
        amount: &[f32],
        bus: &mut [[f32; BLOCK]; 2],
    ) {
        let (table_a, table_b) = (wave_a.table(self.table_a), wave_b.table(self.table_b));
        // VEL scales the EG depth for the Filter and Osc. Mod. destinations only.
        let velocity = (self.velocity + self.slide[2]).clamp(0.0, 1.0);
        let depth = if c.velocity_depth { velocity } else { 1.0 };
        let sync = c.mod_kind == SYNC;
        for (i, &dither) in dither.iter().enumerate() {
            let amp = self.amp.step(&c.amp);
            let e_filter = self.filter_env.step(&c.filter_env);
            let e_mod = self.mod_env.step(&c.mod_env);
            let eg = self.eg.step(&c.eg) * depth;
            // m = clamp(smoothed(v15 + MW) + (2v19 − 1)·E_mod + [EG→Osc.Mod] + [LFO→MOD], 0, 1).
            let m = (amount[i] + c.mod_contour * e_mod + c.eg_mod * eg + c.lfo_mod * lfo[i])
                .clamp(0.0, 1.0);
            let active = m > MOD_THRESHOLD;
            let (fm, ring) = (active && c.mod_kind == FM, active && c.mod_kind == RING);
            let (mut left, mut right, mut b0) = (0.0, 0.0, 0.0);
            for k in 0..c.copies {
                let b = read(table_b, self.phase_b[k], dither);
                // FM: A is read at φ_A + (B + 1)·m cycles.
                let phase_a = if fm {
                    self.phase_a[k].wrapping_add(cycles((b + 1.0) * m))
                } else {
                    self.phase_a[k]
                };
                let mut a = read(table_a, phase_a, dither);
                if ring {
                    a *= 1.0 + 3.0 * m * b;
                }
                let x = c.gain_a * a + c.gain_b * b;
                left += x * c.copy_pan[k][0];
                right += x * c.copy_pan[k][1];
                if k == 0 {
                    b0 = b;
                }
                // Sync: A's phase resets to 0 whenever B's phase wraps.
                let (phase_b, wrapped) = self.phase_b[k].overflowing_add(self.step_b[k]);
                self.phase_b[k] = phase_b;
                self.phase_a[k] = if sync && wrapped {
                    0
                } else {
                    self.phase_a[k].wrapping_add(self.step_a[k])
                };
            }
            let noise = c.noise * white(&mut self.noise);
            // c = clamp(S + (2v27 − 1)³·E_flt + [EG→Filter] + [LFO→FLT] + [Filter mod], 0, 1).
            self.s += CUTOFF_SMOOTHING * (self.s_target - self.s);
            let filter_mod = if active && c.mod_kind == FILTER {
                0.5 * m * b0
            } else {
                0.0
            };
            let cutoff = (self.s
                + c.filter_contour * e_filter
                + c.eg_filter * eg
                + c.lfo_filter * lfo[i]
                + filter_mod)
                .clamp(0.0, 1.0);
            let (a, b) = filter.at(cutoff);
            let feedback = c.resonance * b;
            let out_l = ladder(
                &mut self.stages[0],
                left + noise,
                a,
                feedback,
                c.filter_kind,
            );
            let out_r = ladder(
                &mut self.stages[1],
                right + noise,
                a,
                feedback,
                c.filter_kind,
            );
            // voice = filter out × amp env × gain × pan; gain = clamp(velocity + slide velocity, 0, 1).
            self.gain += SMOOTHING * (velocity - self.gain);
            let gain = amp * self.gain;
            bus[0][i] += out_l * gain * self.pan[0];
            bus[1][i] += out_r * gain * self.pan[1];
            // Idle once the amp env is below 10⁻⁴ while aiming at 0: released, or decaying to sustain 0.
            let to_zero = self.amp.stage == Stage::Release
                || (self.amp.stage == Stage::Decay && c.amp.sustain == 0.0);
            if to_zero && amp < FREE_LEVEL {
                self.amp.stage = Stage::Idle;
                return;
            }
        }
    }
}

/// One channel of the filter: u = clip(x − v22·b·y8[n−1]), eight stages y += a·(u − y). LP 12 is stage 4, LP 24
/// stage 8, HP the clipped input minus stage 1.
fn ladder(y: &mut [f32; STAGES], x: f32, a: f32, feedback: f32, kind: u8) -> f32 {
    let u = x - feedback * y[STAGES - 1];
    let u = u / (1.0 + CLIP * u.abs());
    let mut input = u;
    for stage in y.iter_mut() {
        *stage += a * (input - *stage);
        input = *stage;
    }
    match kind {
        0 => y[3],
        1 => y[STAGES - 1],
        _ => u - y[0],
    }
}

/// The 32-bit phase step of `freq`·`ratio` Hz.
fn phase_step(freq: f32, ratio: f64, sample_rate: f32) -> u32 {
    (f64::from(freq) * ratio / f64::from(sample_rate) * 4_294_967_296.0) as u64 as u32
}

/// `x` cycles as a 32-bit phase offset, wrapped.
fn cycles(x: f32) -> u32 {
    (f64::from(x) * 4_294_967_296.0) as i64 as u32
}

/// Xorshift32.
fn next(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// White noise, uniform in [−1, 1).
fn white(state: &mut u32) -> f32 {
    (next(state) >> 8) as f32 / (1 << 23) as f32 - 1.0
}

/// GMSynth. Build it on the app thread; every [`Instrument`] method is real-time safe.
pub struct GmSynth {
    sample_rate: f32,
    params: [f32; PARAMS],
    voices: [Voice; VOICES],
    /// The key table: keys down, for MONO's fallback and AUTO.
    held: [bool; 128],
    /// The previously played key, where the next note's glide starts (none after reset).
    last_key: Option<u8>,
    serial: u64,
    wave_a: Option<Arc<Wavetable>>,
    wave_b: Option<Arc<Wavetable>>,
    filter: &'static FilterTable,
    /// Channel Pitch Bend v and mod wheel (CC #1) v.
    bend: f32,
    wheel: f32,
    /// `CSTM`: PB RANGE R in semitones, MW AMOUNT, MW DEST item.
    pb_range: f32,
    mw_amount: f32,
    mw_dest: usize,
    control: Control,
    /// Samples into the current block.
    block_pos: usize,
    /// The LFO phase in cycles: free-running while stopped, else the song's.
    lfo_phase: f64,
    /// Smoothed v15 + MW, master level and pan; `snap` sets them to their targets at the next block (after reset).
    mod_amount: f32,
    level: f32,
    pan: f32,
    snap: bool,
    /// Dither and random phases.
    rng: u32,
    /// FX 1–3: a processor bank each, and the kind each enables this block (none while its Enable is 0).
    fx: [Bank; FX_SLOTS],
    fx_kinds: [Option<usize>; FX_SLOTS],
}

impl GmSynth {
    /// A GMSynth at `sample_rate` (44.1 kHz if that is not positive) with the default params and settings. Builds
    /// the default shape and the filter table on first use.
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            TABLE_RATE as f32
        };
        let mut synth = Self {
            sample_rate,
            params: DEFAULTS,
            voices: [Voice::default(); VOICES],
            held: [false; 128],
            last_key: None,
            serial: 0,
            wave_a: shape(DEFAULT_SHAPE),
            wave_b: shape(DEFAULT_SHAPE),
            filter: FILTER_TABLE.get_or_init(FilterTable::build),
            bend: 0.5,
            wheel: 0.0,
            pb_range: PB_RANGES[1],
            mw_amount: 0.5,
            mw_dest: MW_OSC_MOD,
            control: Control::default(),
            block_pos: 0,
            lfo_phase: 0.0,
            mod_amount: 0.0,
            level: 0.0,
            pan: 0.5,
            snap: true,
            rng: SEED,
            fx: std::array::from_fn(|_| Bank::new(sample_rate)),
            fx_kinds: [None; FX_SLOTS],
        };
        synth.reset();
        synth
    }

    fn beats_per_frame(&self, context: &ProcessContext) -> f64 {
        f64::from(context.tempo_bpm.max(0.0)) / (60.0 * f64::from(self.sample_rate))
    }

    /// The LFO phase at `frame` of the call: the song's while it plays, else the free-running one.
    fn lfo_phase_at(&self, context: &ProcessContext, frame: usize) -> f64 {
        if context.playing {
            let beat = context.start_beat + frame as f64 * self.beats_per_frame(context);
            (beat / f64::from(tempo_step(&GMSYNTH_RATE_BEATS, self.params[32]))).rem_euclid(1.0)
        } else {
            self.lfo_phase
        }
    }

    /// The block laws, at `frame` of the call.
    fn control(&mut self, context: &ProcessContext, frame: usize) {
        // FX LFOs: frac(song beats / R_eff) at the block start while the song plays, else free-running. The FX block
        // is this 32-sample control block.
        let start_beat = if context.playing {
            context.start_beat + frame as f64 * self.beats_per_frame(context)
        } else {
            -1.0
        };
        for (slot, (bank, kind)) in self.fx.iter_mut().zip(&mut self.fx_kinds).enumerate() {
            let settings;
            (*kind, settings) = fx_slot(&self.params, slot);
            bank.prepare_block(&settings, context.tempo_bpm, start_beat, BLOCK);
        }
        let lfo_shape = ((2.0 * self.params[33]) as u8).min(2);
        let lfo_now = lfo_value(lfo_shape, self.lfo_phase_at(context, frame));
        let v = &self.params;
        let sr = self.sample_rate;
        let c = &mut self.control;
        let mw = self.wheel * self.mw_amount;
        let mw_at = |dest: usize| if self.mw_dest == dest { mw } else { 0.0 };
        let (mw_filter, mw_lfo, mw_osc) = (mw_at(MW_FILTER), mw_at(MW_LFO), mw_at(MW_OSC_MOD));

        c.amp = Rates {
            attack: attack_rate(v[3], sr),
            decay: decay_rate(v[4], sr),
            sustain: v[5] * v[5],
            release: RATE_SCALE / (sr * (v[6] + 0.001)),
        };
        c.mod_env = Rates {
            attack: attack_rate(v[17], sr),
            decay: decay_rate(v[18], sr),
            ..Rates::default()
        };
        c.filter_env = Rates {
            attack: attack_rate(v[25], sr),
            decay: decay_rate(v[26], sr),
            ..Rates::default()
        };
        c.eg = Rates {
            attack: attack_rate(v[36], sr),
            decay: decay_rate(v[37], sr),
            sustain: v[38] * v[38],
            release: RATE_SCALE / (sr * (200.0 * v[39].powi(4) + 0.001)),
        };

        // Choir: N = int(3·v42) + 1; copy i at 2^((D·i/(N−1) − D/2)/12), odd i ×1.5 (Chord) or ×2 (Octave) by
        // int(2·v43); x_i = v45·(2i/(N−1) − 1), L = clamp(1 − x), R = clamp(1 + x). N = 1 sits at the centre, as its
        // x = 0, and g = 0.5·(1 − v20²).
        c.copies = ((3.0 * v[42]) as usize + 1).min(COPIES);
        let spread = v[44];
        let scale = [1.0, 1.5, 2.0][((2.0 * v[43]) as usize).min(2)];
        for k in 0..COPIES {
            let t = if c.copies > 1 {
                k as f32 / (c.copies - 1) as f32
            } else {
                0.5
            };
            let odd = if k % 2 == 1 { scale } else { 1.0 };
            c.ratio[k] = f64::from(((spread * t - spread / 2.0) / 12.0).exp2()) * odd;
            let x = v[45] * (2.0 * t - 1.0);
            c.copy_pan[k] = [(1.0 - x).clamp(0.0, 1.0), (1.0 + x).clamp(0.0, 1.0)];
        }
        // g = 0.5/(1 + 0.15·(N − 1)) × (1 − v20²); A g·(1 − |2v13 − 1|), B g·(2v13 − 1); noise v20².
        let g = 0.5 / (1.0 + 0.15 * (c.copies - 1) as f32) * (1.0 - v[20] * v[20]);
        c.gain_a = g * (1.0 - (2.0 * v[13] - 1.0).abs());
        c.gain_b = g * (2.0 * v[13] - 1.0);
        c.noise = v[20] * v[20];
        c.ratio_b = ratio_b(v);
        c.offset = transpose_step(v[28]) + self.pb_range * (2.0 * self.bend - 1.0);
        c.slide = v[29];

        // EG depth c = 2v40 − 1 by int(2·v41): Filter c³, Osc. Mod |c|·c, Pitch 12·c·|c| st.
        let depth = 2.0 * v[40] - 1.0;
        let eg_dest = ((2.0 * v[41]) as usize).min(2);
        c.eg_filter = if eg_dest == 0 { depth.powi(3) } else { 0.0 };
        c.eg_mod = if eg_dest == 1 {
            depth.abs() * depth
        } else {
            0.0
        };
        c.eg_pitch = if eg_dest == 2 {
            12.0 * depth * depth.abs()
        } else {
            0.0
        };
        // VEL: any nonzero value is on.
        c.velocity_depth = v[74] != 0.0;

        // LFO: A = 2v34 − 1, MW→LFO adds to |A|, clamp ±1; depth by int(4·v35): FLT A³, MOD |A|A, FREQ 12·A|A| st,
        // LVL ±12·A dB, PAN |A|A.
        let a = 2.0 * v[34] - 1.0;
        let a = (a.abs() + mw_lfo).min(1.0).copysign(a);
        c.lfo_shape = lfo_shape;
        let lfo_dest = ((4.0 * v[35]) as usize).min(4);
        let at = |dest: usize, value: f32| if lfo_dest == dest { value } else { 0.0 };
        c.lfo_filter = at(0, a.powi(3));
        c.lfo_mod = at(1, a.abs() * a);
        c.lfo_mod_now = c.lfo_mod * lfo_now;
        c.lfo_pitch = at(2, 12.0 * a * a.abs()) * lfo_now;
        let lfo_level = at(3, 12.0 * a) * lfo_now;
        let lfo_pan = at(4, a.abs() * a) * lfo_now;

        c.mod_kind = ((3.0 * v[16]) as u8).min(3);
        c.mod_contour = 2.0 * v[19] - 1.0;
        c.mod_target = v[15] + mw_osc;
        c.sync_base = c.mod_target.clamp(0.0, 1.0);

        c.filter_contour = (2.0 * v[27] - 1.0).powi(3);
        c.cutoff = v[21];
        c.kbd = v[24];
        c.mw_filter = mw_filter;
        c.resonance = v[22];
        c.filter_kind = ((2.0 * v[23]) as u8).min(2);

        // Master: level dB = 66·v1 − 48 (+ LFO), 0 below −60 dB; P = clamp(v2 + LFO, 0, 1).
        let db = 66.0 * v[1] - 48.0 + lfo_level;
        c.level = if db < SILENT_DB {
            0.0
        } else {
            10.0_f32.powf(db / 20.0)
        };
        c.pan = (v[2] + lfo_pan).clamp(0.0, 1.0);
        if self.snap {
            (self.level, self.pan, self.mod_amount) = (c.level, c.pan, c.mod_target);
            self.snap = false;
        }

        for voice in self.voices.iter_mut().filter(|voice| voice.sounding()) {
            voice.glide(self.control.slide, sr);
            voice.control(&self.control, sr);
        }
    }

    /// Renders `left`/`right`, which lie inside one block, starting at `frame` of the call.
    fn render(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        context: &ProcessContext,
        frame: usize,
    ) {
        let n = left.len();
        let c = self.control;
        let (mut dither, mut lfo, mut amount) =
            ([0_u32; BLOCK], [0.0_f32; BLOCK], [0.0_f32; BLOCK]);
        let step = self.beats_per_frame(context)
            / f64::from(tempo_step(&GMSYNTH_RATE_BEATS, self.params[32]));
        for i in 0..n {
            // One dither per sample, shared by both oscillators, every copy and every voice.
            dither[i] = next(&mut self.rng) >> 10;
            let phase = self.lfo_phase_at(context, frame + i);
            self.lfo_phase = (phase + step).fract();
            lfo[i] = lfo_value(c.lfo_shape, phase);
            self.mod_amount += SMOOTHING * (c.mod_target - self.mod_amount);
            amount[i] = self.mod_amount;
        }
        let mut bus = [[0.0_f32; BLOCK]; 2];
        // A missing shape for A or B silences the voices; they hold until it is back.
        if let (Some(a), Some(b)) = (&self.wave_a, &self.wave_b) {
            for voice in self.voices.iter_mut().filter(|voice| voice.sounding()) {
                let (dither, lfo, amount) = (&dither[..n], &lfo[..n], &amount[..n]);
                voice.render(&c, (a, b), self.filter, dither, lfo, amount, &mut bus);
            }
        }
        for i in 0..n {
            // FX 1 → FX 2 → FX 3 in series, then the master level and pan.
            let mut out = [bus[0][i], bus[1][i]];
            for (bank, &kind) in self.fx.iter_mut().zip(&self.fx_kinds) {
                out = bank.process_frame(out, kind);
            }
            self.level += SMOOTHING * (c.level - self.level);
            self.pan += SMOOTHING * (c.pan - self.pan);
            let (l, r) = ((2.0 - 2.0 * self.pan).min(1.0), (2.0 * self.pan).min(1.0));
            left[i] += out[0] * self.level * l * l;
            right[i] += out[1] * self.level * r * r;
        }
    }
}

/// FX slot `slot`'s enabled kind and settings. Enable is on at any v ≠ 0, 0.5 included. Kind
/// and Sync trunc(9v) (DIST … VOX; R = [1/4 … 32] beats), LFO amount sign(2v − 1)·(2v − 1)², shape trunc(2v) (≥ 2
/// Square), X and Y raw.
fn fx_slot(v: &[f32; PARAMS], slot: usize) -> (Option<usize>, Settings) {
    let p = &v[FX_PARAMS + 7 * slot..][..7];
    let menu = |v: f32, n: usize| ((v.clamp(0.0, 1.0) * n as f32) as usize).min(n);
    let amount = 2.0 * p[4] - 1.0;
    let settings = Settings {
        x: p[5],
        y: p[6],
        rate_beats: GMSYNTH_RATE_BEATS[menu(p[2], 9)],
        amount: amount * amount.abs(),
        shape: menu(p[3], 2),
    };
    ((p[1] != 0.0).then_some(menu(p[0], 9)), settings)
}

/// f_B/f_A for PITCH B ±24 st and FINE B ±0.5 st, all in f64: s = trunc(f64(v11)·48 − 24), exact, then
/// f32(exp2(((f64(v12) − 0.5) + s)·(1/12))). The `%d` text shows this step too ([`pitch_b_step`]), so the text always
/// matches the sound.
fn ratio_b(v: &[f32; PARAMS]) -> f32 {
    let s = pitch_b_step(v[11]);
    (((f64::from(v[12]) - 0.5) + s) * (1.0 / 12.0)).exp2() as f32
}

/// PITCH B (p11) in semitones: s = trunc(f64(v)·48 − 24), exact (the Slicer law).
pub fn pitch_b_step(v: f32) -> f64 {
    (f64::from(v) * 48.0 - 24.0).trunc()
}

/// TRNSP: s = trunc(fma_f32(v28, 48, −24)), one fused multiply-add then truncation (the DW Sampler law).
/// The `%d` text uses it too.
pub fn transpose_step(v: f32) -> f32 {
    48.0_f32.mul_add(v, -24.0).trunc()
}

/// LFO shapes by int(2·v33): Sine sin 2πφ, Saw 2φ − 1, Square −1 below φ = 0.5, else +1.
fn lfo_value(shape: u8, phase: f64) -> f32 {
    match shape {
        0 => (TAU * phase as f32).sin(),
        1 => (2.0 * phase - 1.0) as f32,
        _ => {
            if phase < 0.5 {
                -1.0
            } else {
                1.0
            }
        }
    }
}

/// The voice pan law: L = 1 − max(0, 2p − 1), R = 1 + min(0, 2p − 1).
fn pan_gains(p: f32) -> [f32; 2] {
    [
        1.0 - (2.0 * p - 1.0).max(0.0),
        1.0 + (2.0 * p - 1.0).min(0.0),
    ]
}

impl Instrument for GmSynth {
    /// Stores every param; hidden 7–10 and 67–73 are never read. Turning MONO on sends a note-off to all 128
    /// keys first.
    fn set_parameter(&mut self, index: usize, value: f32) {
        if index == SINGLE_VOICE && value != 0.0 && self.params[SINGLE_VOICE] == 0.0 {
            for key in 0..128 {
                self.note_off(key);
            }
        }
        if let Some(param) = self.params.get_mut(index) {
            *param = value;
        }
    }

    /// Ignored while Enable < 0.5. Poly: the highest-numbered free voice, else the oldest-started is stolen.
    /// MONO: voice 0, legato (key and velocity only) while it sounds unreleased. A note arriving
    /// mid-block takes its glide step and block values at once; one at a block start waits for that block's laws.
    fn note_on(&mut self, key: u8, velocity: f32, pan: f32) {
        if self.params[ENABLE] < 0.5 || key > 127 {
            return;
        }
        let others_held = self
            .held
            .iter()
            .enumerate()
            .any(|(k, &down)| down && k != usize::from(key));
        self.held[usize::from(key)] = true;
        let velocity = velocity.clamp(0.0, 1.0);
        let mono = self.params[SINGLE_VOICE] != 0.0;
        let glide_from = match self.last_key.replace(key) {
            // AUTO (v31, on at any nonzero v): no glide unless another key is held.
            Some(last) if self.params[31] == 0.0 || others_held => f32::from(last),
            _ => f32::from(key),
        };
        if mono && self.voices[0].held() {
            let voice = &mut self.voices[0];
            voice.key = key;
            voice.velocity = velocity;
            return;
        }
        let index = if mono {
            0
        } else {
            (0..VOICES)
                .rev()
                .find(|&i| !self.voices[i].sounding())
                .or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age))
                .unwrap_or(0)
        };
        self.serial += 1;
        let ratio_b = f64::from(ratio_b(&self.params));
        let phase_offset = f64::from(self.params[14]);
        let voice = &mut self.voices[index];
        if !voice.sounding() {
            // A voice reset from idle: random A phases, a clear filter, and its gain snaps to the velocity; a voice
            // still sounding keeps its gain and glides 1 % per sample.
            for phase in &mut voice.phase_a {
                *phase = next(&mut self.rng);
            }
            voice.stages = [[0.0; STAGES]; 2];
            voice.gain = velocity;
        }
        // B locks to A at retrigger: φ_B = frac(v14 + φ_A·f_B/f_A).
        for (b, &a) in voice.phase_b.iter_mut().zip(&voice.phase_a) {
            let cycles = phase_offset + f64::from(a) / 4_294_967_296.0 * ratio_b;
            *b = (cycles.fract() * 4_294_967_296.0) as u32;
        }
        let pan = pan.clamp(0.0, 1.0);
        voice.key = key;
        voice.velocity = velocity;
        voice.note_pan = pan;
        voice.pan = pan_gains(pan);
        voice.slide = [0.0; 3];
        voice.age = self.serial;
        voice.p = glide_from;
        // The amp level continues from where it is; the other envelopes restart from 0.
        voice.amp.stage = Stage::Attack;
        voice.filter_env.restart();
        voice.mod_env.restart();
        voice.eg.restart();
        voice.snap = true;
        if self.block_pos != 0 {
            voice.glide(self.control.slide, self.sample_rate);
            voice.control(&self.control, self.sample_rate);
        }
    }

    /// Poly: releases the voices holding `key`. MONO, the sounding key: falls back to the highest held key below
    /// it, else the lowest above, without a retrigger; none held, it releases.
    fn note_off(&mut self, key: u8) {
        let Some(down) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        *down = false;
        if self.params[SINGLE_VOICE] == 0.0 {
            for voice in self.voices.iter_mut().filter(|voice| voice.key == key) {
                if voice.held() {
                    voice.release();
                }
            }
            return;
        }
        let voice = &mut self.voices[0];
        if voice.held() && voice.key == key {
            let held = |k: &u8| self.held[usize::from(*k)];
            match (0..key)
                .rev()
                .find(held)
                .or_else(|| (key + 1..128).find(held))
            {
                Some(fallback) => voice.key = fallback,
                None => voice.release(),
            }
        }
    }

    fn all_notes_off(&mut self) {
        self.held = [false; 128];
        for voice in &mut self.voices {
            voice.release();
        }
    }

    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        for (seed, voice) in (1..).zip(&mut self.voices) {
            voice.noise = SEED.rotate_left(seed * 8) ^ seed;
        }
        self.held = [false; 128];
        self.last_key = None;
        self.serial = 0;
        self.block_pos = 0;
        self.lfo_phase = 0.0;
        self.snap = true;
        self.rng = SEED;
        for bank in &mut self.fx {
            bank.reset();
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32], context: ProcessContext) {
        let frames = left.len().min(right.len());
        let mut at = 0;
        while at < frames {
            if self.block_pos == 0 {
                self.control(&context, at);
            }
            let n = (BLOCK - self.block_pos).min(frames - at);
            self.render(&mut left[at..at + n], &mut right[at..at + n], &context, at);
            self.block_pos = (self.block_pos + n) % BLOCK;
            at += n;
        }
    }

    /// Pitch Bend: w = 2v − 1 times R; MIDI CC #1: the mod wheel.
    fn set_controller(&mut self, index: usize, value: f32) {
        match index {
            PITCH_BEND => self.bend = value,
            MOD_WHEEL => self.wheel = value,
            _ => {}
        }
    }

    /// Slides add to every voice's pitch (from the next block), velocity gain (smoothed as a velocity is) and pan.
    fn set_slide(&mut self, pitch: f32, pan: f32, velocity: f32) {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        for voice in &mut self.voices {
            voice.slide = [finite(pitch), finite(pan), finite(velocity)];
            voice.pan = pan_gains((voice.note_pan + voice.slide[1]).clamp(0.0, 1.0));
        }
    }

    /// The `CSTM` settings. A shape is taken from the cache only; [`prepare_setting`] builds it on the app thread.
    fn set_setting(&mut self, key: u32, value: f32) {
        match key {
            WAVE_A | WAVE_B => {
                let slot = value as usize;
                debug_assert!(
                    slot >= SLOTS || cached(slot).is_some(),
                    "prepare_setting builds slot {slot} first"
                );
                *if key == WAVE_A {
                    &mut self.wave_a
                } else {
                    &mut self.wave_b
                } = cached(slot);
            }
            PB_RANGE => self.pb_range = PB_RANGES[(value as usize).min(PB_RANGES.len() - 1)],
            MW_AMOUNT => self.mw_amount = value.clamp(0.0, 1.0),
            MW_DEST => self.mw_dest = (value as usize).min(MW_OSC_MOD),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;
    const STOPPED: ProcessContext = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };

    fn run(synth: &mut GmSynth, frames: usize) {
        let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
        synth.process(&mut left, &mut right, STOPPED);
    }

    fn keys(synth: &GmSynth) -> Vec<Option<u8>> {
        (synth.voices.iter())
            .map(|voice| voice.sounding().then_some(voice.key))
            .collect()
    }

    #[test]
    fn filter_table_follows_the_recipe() {
        let table = FILTER_TABLE.get_or_init(FilterTable::build);
        // b: 1.888 at the low end, 9.86 at the top.
        assert!((table.0[0][1] - 1.888).abs() < 0.001, "{}", table.0[0][1]);
        let top = table.0[FILTER_TABLE_LEN - 1][1];
        assert!((top - 9.86).abs() < 0.001, "{top}");
        // 8·arg H − ω = −π and b·|H|⁸ = 1 at f_r, for the c the cutoff law maps there.
        for c in [0.0_f32, 0.2, 0.5357, 0.8, 0.99] {
            let (a, b) = table.at(c);
            let f_r =
                f64::from(PEAK_SCALE * CUTOFF_HZ * (CUTOFF_OCTAVES * c).exp2() + PEAK_OFFSET_HZ);
            let w = TAU_64 * f_r / TABLE_RATE;
            let (a, b) = (f64::from(a), f64::from(b));
            let (re, im) = (1.0 - (1.0 - a) * w.cos(), (1.0 - a) * w.sin());
            let phase = 8.0 * -im.atan2(re) - w;
            assert!((phase + PI).abs() < 1e-4, "c {c}: phase {phase}");
            let gain = b * (a / re.hypot(im)).powi(8);
            assert!((gain - 1.0).abs() < 1e-4, "c {c}: loop gain {gain}");
        }
    }

    #[test]
    fn poly_takes_the_highest_free_voice_and_steals_the_oldest() {
        let mut synth = GmSynth::new(RATE);
        for key in [60, 62, 64] {
            synth.note_on(key, 1.0, 0.5);
        }
        assert_eq!(keys(&synth), [None, Some(64), Some(62), Some(60)]);
        synth.note_on(65, 1.0, 0.5);
        // All four sound: the oldest-started (60, voice 3) goes, released or not.
        synth.note_off(62);
        synth.note_on(67, 1.0, 0.5);
        assert_eq!(keys(&synth), [Some(65), Some(64), Some(62), Some(67)]);
        synth.note_on(69, 1.0, 0.5);
        assert_eq!(keys(&synth), [Some(65), Some(64), Some(69), Some(67)]);
    }

    #[test]
    fn mono_is_legato_on_voice_zero_and_falls_back_without_a_retrigger() {
        let mut synth = GmSynth::new(RATE);
        synth.set_parameter(25, 0.5); // a filter env attack, to see restarts
        synth.note_on(50, 1.0, 0.5);
        synth.set_parameter(SINGLE_VOICE, 1.0);
        // Turning MONO on sent note-offs to every key.
        assert_eq!(synth.voices[3].amp.stage, Stage::Release);
        assert!(!synth.held.iter().any(|&down| down));
        synth.note_on(60, 0.5, 0.5);
        assert_eq!(keys(&synth)[0], Some(60));
        run(&mut synth, 4800);
        let before = synth.voices[0].filter_env.e;
        assert!(before > 0.1);
        // Legato: key and velocity change, nothing restarts.
        synth.note_on(64, 0.9, 0.2);
        let voice = synth.voices[0];
        assert_eq!((voice.key, voice.velocity), (64, 0.9));
        assert_eq!(voice.filter_env.e, before);
        assert_eq!(voice.pan, [1.0, 1.0], "legato keeps the pan");
        synth.note_on(55, 1.0, 0.5);
        synth.note_off(55);
        // Highest held below 55: none; lowest held above: 60.
        assert_eq!(synth.voices[0].key, 60);
        assert_eq!(synth.voices[0].filter_env.e, before);
        synth.note_off(60);
        assert_eq!(synth.voices[0].key, 64);
        synth.note_off(64);
        assert_eq!(synth.voices[0].amp.stage, Stage::Release);
        // Released: the next note retriggers.
        synth.note_on(62, 1.0, 0.5);
        assert_eq!(synth.voices[0].filter_env.e, 0.0);
        assert_eq!(synth.voices[0].amp.stage, Stage::Attack);
    }

    #[test]
    fn glide_starts_at_the_previous_note_and_closes_one_pole() {
        let mut synth = GmSynth::new(RATE);
        synth.set_parameter(29, 0.5);
        synth.note_on(48, 1.0, 0.5);
        synth.note_off(48);
        synth.note_on(60, 1.0, 0.5);
        let voice = |synth: &GmSynth| synth.voices[2];
        assert_eq!(voice(&synth).p, 48.0);
        // p += (target − p)·32/(0.1·sr·v29 + 1) per block.
        let k = BLOCK as f32 / (0.1 * RATE * 0.5 + 1.0);
        run(&mut synth, 10 * BLOCK);
        let expected = 60.0 - 12.0 * (1.0 - k).powi(10);
        assert!(
            (voice(&synth).p - expected).abs() < 1e-3,
            "{}",
            voice(&synth).p
        );
        // AUTO: no glide unless another key is held.
        let mut auto = GmSynth::new(RATE);
        auto.set_parameter(29, 0.5);
        auto.set_parameter(31, 0.001); // on at any nonzero v
        auto.note_on(48, 1.0, 0.5);
        auto.note_off(48);
        auto.note_on(60, 1.0, 0.5);
        assert_eq!(auto.voices[2].p, 60.0);
        auto.note_on(67, 1.0, 0.5);
        assert_eq!(auto.voices[1].p, 60.0);
        // Slide 0 snaps at the first block.
        let mut snap = GmSynth::new(RATE);
        snap.note_on(48, 1.0, 0.5);
        snap.note_on(60, 1.0, 0.5);
        run(&mut snap, 1);
        assert_eq!(snap.voices[2].p, 60.0);
    }

    #[test]
    fn a_voice_idles_once_its_amp_env_aims_at_zero_below_1e_4() {
        // Idle when the amp env is < 10⁻⁴ with target 0, a decay to sustain 0 included (no note-off).
        let mut synth = GmSynth::new(RATE);
        synth.set_parameter(4, 0.0);
        synth.set_parameter(5, 0.0);
        synth.note_on(60, 1.0, 0.5);
        assert_eq!(keys(&synth), [None, None, None, Some(60)]);
        run(&mut synth, 480);
        assert_eq!(keys(&synth), [None; VOICES]);
        synth.note_on(62, 1.0, 0.5);
        assert_eq!(keys(&synth)[3], Some(62));
        // Sustain above 0 keeps it.
        synth.set_parameter(5, 0.5);
        synth.note_on(64, 1.0, 0.5);
        run(&mut synth, 480);
        assert_eq!(keys(&synth)[2], Some(64));
    }

    #[test]
    fn semitone_steps_follow_the_round_12_laws() {
        // At v = f32(k/48). TRNSP: trunc(fma_f32(v, 48, −24)), the DW table. PITCH B audio: trunc(f64(v)·48 − 24),
        // the Slicer table.
        let transpose_off = [
            (16, -7),
            (19, -4),
            (22, -1),
            (25, 0),
            (28, 3),
            (31, 6),
            (34, 9),
            (37, 12),
            (40, 15),
        ];
        for k in 0..=48_i32 {
            let v = (f64::from(k) / 48.0) as f32;
            let transpose = transpose_off
                .iter()
                .find(|o| o.0 == k)
                .map_or(k - 24, |o| o.1);
            assert_eq!(transpose_step(v), transpose as f32, "TRNSP k {k}");
            let pitch_b = match k {
                1 | 2 | 4 | 8 | 11 | 13 | 16 | 19 | 22 => k - 23,
                25.. if (k - 25) % 3 == 0 => k - 25,
                _ => k - 24,
            };
            let mut params = DEFAULTS;
            params[11] = v;
            let expected = (f64::from(pitch_b) / 12.0).exp2() as f32;
            assert_eq!(ratio_b(&params), expected, "PITCH B k {k}");
        }
    }

    #[test]
    fn a_decay_to_zero_ends_at_zero() {
        let mut env = Envelope {
            e: 1.0,
            stage: Stage::Decay,
        };
        let rates = Rates {
            decay: decay_rate(0.0, RATE),
            ..Rates::default()
        };
        // k = 0.18: 1e-20 after 226 samples, while plain f32 would only underflow after about 500.
        let level = (0..300).map(|_| env.step(&rates)).last().unwrap();
        assert_eq!(level, 0.0);
    }

    #[test]
    fn b_locks_to_a_at_retrigger() {
        let mut synth = GmSynth::new(RATE);
        synth.set_parameter(14, 0.25);
        synth.set_parameter(11, 0.75); // +12 st: f_B/f_A = 2
        synth.note_on(60, 1.0, 0.5);
        let voice = synth.voices[3];
        for (a, b) in voice.phase_a.iter().zip(voice.phase_b) {
            let expected = (0.25 + f64::from(*a) / 4_294_967_296.0 * 2.0).fract();
            let found = f64::from(b) / 4_294_967_296.0;
            assert!((found - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn fx_slots_read_their_params_by_the_round_8_laws() {
        use crate::multifx::{DELAY, FLANGER, LPF, VOX};
        let kinds = |v: &[f32; PARAMS]| [0, 1, 2].map(|slot| fx_slot(v, slot).0);
        // The defaults 1/9, 5/9 and 3/9 (exact), all off. Enable is on at any v ≠ 0:
        // 0.001 and 0.5 included.
        let mut v = DEFAULTS;
        assert_eq!(kinds(&v), [None; 3]);
        for on in [0.001, 0.5, 1.0] {
            (v[47], v[54], v[61]) = (on, on, on);
            assert_eq!(
                kinds(&v),
                [Some(FLANGER), Some(DELAY), Some(LPF)],
                "Enable {on}"
            );
        }
        // kind = trunc(9v), plain: k/9 is step k, and just below it is step k − 1.
        for k in 1..=9 {
            v[53] = k as f32 / 9.0;
            assert_eq!(fx_slot(&v, 1).0, Some(k), "kind {k}/9");
            v[53] = k as f32 / 9.0 - 1e-4;
            assert_eq!(fx_slot(&v, 1).0, Some(k - 1), "kind {k}/9 − 1e-4");
        }
        (v[46], v[53], v[60]) = (1.0, 0.99, 0.0);
        assert_eq!(kinds(&v), [Some(VOX), Some(8), Some(0)]);
        // Sync R by trunc(9v), shape trunc(2v) (≥ 2 Square), amount sign(2v − 1)·(2v − 1)², X and Y raw.
        for (sync, rate, shape_v, shape, amount_v, amount) in [
            (0.0, 0.25, 0.0, 0, 0.5, 0.0),
            (1.0 / 9.0, 0.375, 0.49, 0, 0.25, -0.25),
            (5.0 / 9.0, 2.0, 0.5, 1, 0.75, 0.25),
            (0.999, 16.0, 0.99, 1, 0.0, -1.0),
            (4.0 / 9.0 - 1e-4, 0.75, 0.4999, 0, 0.5, 0.0),
            (4.0 / 9.0, 1.0, 0.5, 1, 0.5, 0.0),
            (1.0, 32.0, 1.0, 2, 0.9, 0.64),
        ] {
            (v[55], v[56], v[57], v[58], v[59]) = (sync, shape_v, amount_v, 0.3, 0.8);
            let (_, settings) = fx_slot(&v, 1);
            let expected = Settings {
                x: 0.3,
                y: 0.8,
                rate_beats: rate,
                amount,
                shape,
            };
            assert!(
                (settings.amount - amount).abs() < 1e-6,
                "{settings:?} != {expected:?}"
            );
            assert_eq!(Settings { amount, ..settings }, expected);
        }
    }
}
