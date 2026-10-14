//! The built-in one-shots: drums synthesized from
//! oscillators, seeded noise, envelopes and filters, 808/909 style. No sample goes in; every number below is our own.
//!
//! - The default kit: a slot whose folder is `Built-in/<Pad>` and that has no sample path plays `builtin(folder)`.
//!   Its eight pads are bit-identical to the first version (pinned by hash in the tests).
//! - The factory kits ([`KITS`]): `Built-in/<Key>/<Pad>`. Each pad is a recipe below with its own numbers
//!   and its own noise seed (the FNV-1a hash of `<Key>/<Pad>`).
//!
//! Details:
//! - Dust's lo-fi chain runs on every Dust pad, in this order: hold every other frame (½ rate, aliasing kept), round to
//!   12-bit steps of the pad's peak, then two ≈7 kHz Butterworth low-passes. The low-pass goes last so the hold's
//!   images above it are removed and the kit stays dark. "Seeded crackle" is the Crackle pad; no other pad has
//!   crackle under it, so every pad still ends in silence.
//! - Foundry's every pad goes through a soft clip (`tanh(2x)` of the pad at peak 1): the "hard" part of its character.
//! - The default kit's Closed and Open Hat share one seed (as they always have); every factory pad that uses noise has
//!   its own (Cowbell and Metal use none).

use std::f32::consts::TAU;

use crate::biquad::{Biquad, Response};
use crate::sample::SampleData;

/// The one-shots' sample rate.
pub const RATE: u32 = 44_100;
/// The built-in pads, in the default kit's pad order (`DrumKit::default_kit`).
pub const PADS: [&str; 8] = [
    "Kick",
    "Snare",
    "Clap",
    "Closed Hat",
    "Open Hat",
    "Tom",
    "Perc",
    "Crash",
];
/// The factory kits as (key, pads in pad order). Pad `<Pad>` of kit `<Key>` plays `builtin("Built-in/<Key>/<Pad>")`.
pub const KITS: [(&str, &[&str]); 4] = [
    // Clean, short and tight.
    ("Pocket", &["Kick", "Snare", "Closed Hat", "Clap"]),
    // Lo-fi: ½-rate hold, 12-bit steps, ≈7 kHz low-pass, seeded crackle.
    (
        "Dust",
        &[
            "Kick",
            "Snare",
            "Rim",
            "Closed Hat",
            "Open Hat",
            "Shaker",
            "Tom",
            "Crackle",
        ],
    ),
    // An analog machine: a long sub kick, tuned toms.
    (
        "Relay",
        &[
            "Kick",
            "Snare",
            "Clap",
            "Closed Hat",
            "Open Hat",
            "Low Tom",
            "Mid Tom",
            "High Tom",
            "Rim",
            "Cowbell",
            "Cymbal",
        ],
    ),
    // Hard and industrial: a driven kick, metallic hits.
    (
        "Foundry",
        &[
            "Kick",
            "Hard Kick",
            "Snare",
            "Clap",
            "Closed Hat",
            "Open Hat",
            "Ride",
            "Crash",
            "Low Tom",
            "High Tom",
            "Rim",
            "Shaker",
            "Metal",
            "Noise Hit",
        ],
    ),
];

const SR: f32 = RATE as f32;
/// −1 dBFS.
const PEAK: f32 = 0.891_250_9;
/// The last 5 ms fade to exact silence.
const FADE: usize = RATE as usize / 200;
/// Inharmonic square-wave stacks (Hz) for the metallic sounds: our own picks.
const HAT_TONES: [f32; 6] = [287.0, 431.0, 497.0, 612.0, 771.0, 853.0];
const CRASH_TONES: [f32; 8] = [341.0, 509.0, 593.0, 747.0, 911.0, 1087.0, 1213.0, 1399.0];

/// The built-in one-shot a slot folder names: `Built-in/<Pad>` with `<Pad>` one of [`PADS`], or
/// `Built-in/<Key>/<Pad>` with a kit and pad of [`KITS`] (`\` or `/` between them). Mono at
/// [`RATE`], deterministic, at most 3 s, peaking at −1 dBFS and ending in silence. Rendered on each call, on the app
/// thread; the sample cache keeps the result. `Built-in/Shapes/<Name>` is a 3x Osc custom shape instead
/// (`builtin_shapes`): a whole looping cycle, so it gets neither the peak pass nor the tail fade. `Built-in/Loops/<Name>`
/// is a Slicer loop (`builtin_loops`), which does its own peak pass and fades only where a hit is cut.
pub fn builtin(folder: &str) -> Option<SampleData> {
    let rel = folder.strip_prefix("Built-in")?.strip_prefix(['/', '\\'])?;
    let mut samples = match rel.split_once(['/', '\\']) {
        None => default_pad(rel)?,
        Some(("Loops", name)) => return crate::builtin_loops::loop_sample(name),
        Some(("Shapes", name)) => return crate::builtin_shapes::shape(name),
        Some((kit, pad)) => kit_pad(kit, pad)?,
    };
    let peak = peak(&samples);
    let len = samples.len();
    for (i, s) in samples.iter_mut().enumerate() {
        *s *= PEAK / peak * ((len - 1 - i) as f32 / (FADE - 1) as f32).min(1.0);
    }
    Some(SampleData {
        sample_rate: RATE,
        left: samples.into(),
        right: None,
        ..Default::default()
    })
}

fn default_pad(pad: &str) -> Option<Vec<f32>> {
    Some(match pad {
        "Kick" => drum(&KICK, 0x4b49_434b),
        "Snare" => snare(&SNARE, 0x534e_4152),
        "Clap" => clap(&CLAP, 0x434c_4150),
        "Closed Hat" => hat(&CLOSED_HAT, 0x4841_5453),
        "Open Hat" => hat(&OPEN_HAT, 0x4841_5453),
        "Tom" => drum(&TOM, 0x544f_4d21),
        "Perc" => bell(&PERC),
        "Crash" => cymbal(&CRASH, 0x4352_5348),
        _ => return None,
    })
}

/// A factory kit's pad, before normalizing. Each arm is that pad's recipe.
fn kit_pad(kit: &str, pad: &str) -> Option<Vec<f32>> {
    let seed = seed(kit, pad);
    let samples = match (kit, pad) {
        ("Pocket", "Kick") => drum(
            &Drum {
                low: 58.0,
                high: 210.0,
                drop: 0.018,
                tau: 0.065,
                drive: 1.4,
                click: 0.4,
                click_hz: 5000.0,
                click_tau: 0.002,
                seconds: 0.55,
            },
            seed,
        ),
        ("Pocket", "Snare") => snare(
            &Snare {
                low: 205.0,
                high: 335.0,
                tone_tau: 0.03,
                hiss_hz: 2400.0,
                dull_hz: 9500.0,
                noise_tau: 0.045,
                seconds: 0.4,
                ..SNARE
            },
            seed,
        ),
        ("Pocket", "Closed Hat") => hat(
            &Hat {
                tune: 1.1,
                noise: 0.4,
                band_hz: 10_000.0,
                hp_hz: 7000.0,
                tau: 0.014,
                seconds: 0.12,
            },
            seed,
        ),
        ("Pocket", "Clap") => clap(
            &Clap {
                hz: 1300.0,
                q: 1.8,
                spread: 0.8,
                tail_tau: 0.045,
                seconds: 0.4,
            },
            seed,
        ),

        ("Dust", "Kick") => drum(
            &Drum {
                low: 49.0,
                high: 125.0,
                drop: 0.04,
                tau: 0.13,
                drive: 2.2,
                click: 0.12,
                click_hz: 1500.0,
                click_tau: 0.004,
                seconds: 1.1,
            },
            seed,
        ),
        ("Dust", "Snare") => snare(
            &Snare {
                low: 170.0,
                high: 245.0,
                tone: 0.6,
                tone_tau: 0.05,
                hiss_hz: 1200.0,
                dull_hz: 6000.0,
                noise: 0.8,
                noise_tau: 0.1,
                seconds: 0.75,
            },
            seed,
        ),
        ("Dust", "Rim") => rim(
            &Rim {
                tones: [420.0, 1550.0],
                tau: 0.014,
                seconds: 0.18,
            },
            seed,
        ),
        ("Dust", "Closed Hat") => hat(
            &Hat {
                tune: 0.85,
                noise: 0.6,
                band_hz: 8000.0,
                hp_hz: 5500.0,
                tau: 0.028,
                seconds: 0.22,
            },
            seed,
        ),
        ("Dust", "Open Hat") => hat(
            &Hat {
                tune: 0.85,
                noise: 0.6,
                band_hz: 8000.0,
                hp_hz: 5500.0,
                tau: 0.19,
                seconds: 1.6,
            },
            seed,
        ),
        ("Dust", "Shaker") => shaker(
            &Shaker {
                band_hz: 6000.0,
                attack: 0.02,
                tau: 0.05,
                seconds: 0.45,
            },
            seed,
        ),
        ("Dust", "Tom") => drum(
            &Drum {
                low: 110.0,
                high: 165.0,
                drop: 0.05,
                tau: 0.11,
                drive: 1.2,
                click: 0.15,
                click_hz: 2500.0,
                seconds: 0.95,
                ..TOM
            },
            seed,
        ),
        ("Dust", "Crackle") => crackle(30.0, 0.3, 1.6, seed),

        ("Relay", "Kick") => drum(
            &Drum {
                low: 45.0,
                high: 135.0,
                drop: 0.045,
                tau: 0.15,
                drive: 1.4,
                click: 0.2,
                click_hz: 2500.0,
                click_tau: 0.003,
                seconds: 1.2,
            },
            seed,
        ),
        ("Relay", "Snare") => snare(
            &Snare {
                low: 190.0,
                high: 330.0,
                tone_tau: 0.05,
                hiss_hz: 2500.0,
                dull_hz: 11_000.0,
                noise: 0.9,
                noise_tau: 0.09,
                seconds: 0.65,
                ..SNARE
            },
            seed,
        ),
        ("Relay", "Clap") => clap(
            &Clap {
                hz: 1150.0,
                spread: 1.1,
                tail_tau: 0.09,
                seconds: 0.7,
                ..CLAP
            },
            seed,
        ),
        ("Relay", "Closed Hat") => hat(
            &Hat {
                tune: 1.25,
                noise: 0.2,
                hp_hz: 7000.0,
                tau: 0.02,
                seconds: 0.16,
                ..CLOSED_HAT
            },
            seed,
        ),
        ("Relay", "Open Hat") => hat(
            &Hat {
                tune: 1.25,
                noise: 0.2,
                hp_hz: 7000.0,
                tau: 0.15,
                seconds: 1.1,
                ..OPEN_HAT
            },
            seed,
        ),
        // Tuned a fourth apart: G2, C3, F3.
        ("Relay", "Low Tom") => drum(&tuned_tom(98.0), seed),
        ("Relay", "Mid Tom") => drum(&tuned_tom(130.8), seed),
        ("Relay", "High Tom") => drum(&tuned_tom(174.6), seed),
        ("Relay", "Rim") => rim(
            &Rim {
                tones: [480.0, 1700.0],
                tau: 0.011,
                seconds: 0.14,
            },
            seed,
        ),
        ("Relay", "Cowbell") => bell(&Bell {
            tones: &[562.0, 835.0],
            band_hz: 2600.0,
            q: 1.0,
            tau: 0.09,
            seconds: 0.65,
        }),
        ("Relay", "Cymbal") => cymbal(
            &Cymbal {
                tune: 1.15,
                metal: 0.8,
                hp_hz: 5000.0,
                tau: 0.38,
                seconds: 2.9,
            },
            seed,
        ),

        ("Foundry", "Kick") => drum(
            &Drum {
                low: 50.0,
                high: 230.0,
                drop: 0.025,
                tau: 0.1,
                drive: 4.0,
                click: 0.5,
                click_hz: 6000.0,
                click_tau: 0.002,
                seconds: 1.0,
            },
            seed,
        ),
        ("Foundry", "Hard Kick") => drum(
            &Drum {
                low: 56.0,
                high: 360.0,
                drop: 0.018,
                tau: 0.11,
                drive: 9.0,
                click: 0.7,
                click_hz: 8000.0,
                click_tau: 0.0025,
                seconds: 1.3,
            },
            seed,
        ),
        ("Foundry", "Snare") => snare(
            &Snare {
                low: 220.0,
                high: 350.0,
                tone: 0.6,
                tone_tau: 0.04,
                hiss_hz: 1500.0,
                dull_hz: 12_000.0,
                noise: 1.0,
                noise_tau: 0.075,
                seconds: 0.65,
            },
            seed,
        ),
        ("Foundry", "Clap") => clap(
            &Clap {
                hz: 1500.0,
                q: 1.2,
                spread: 1.4,
                tail_tau: 0.075,
                seconds: 0.75,
            },
            seed,
        ),
        ("Foundry", "Closed Hat") => hat(
            &Hat {
                tune: 1.6,
                noise: 0.5,
                band_hz: 8500.0,
                hp_hz: 6000.0,
                tau: 0.022,
                seconds: 0.2,
            },
            seed,
        ),
        ("Foundry", "Open Hat") => hat(
            &Hat {
                tune: 1.6,
                noise: 0.5,
                band_hz: 8500.0,
                hp_hz: 6000.0,
                tau: 0.2,
                seconds: 1.7,
            },
            seed,
        ),
        ("Foundry", "Ride") => cymbal(
            &Cymbal {
                tune: 1.5,
                metal: 2.0,
                hp_hz: 2800.0,
                tau: 0.34,
                seconds: 2.8,
            },
            seed,
        ),
        ("Foundry", "Crash") => cymbal(
            &Cymbal {
                tune: 0.9,
                metal: 0.8,
                hp_hz: 3500.0,
                tau: 0.36,
                seconds: 3.0,
            },
            seed,
        ),
        ("Foundry", "Low Tom") => drum(
            &Drum {
                low: 85.0,
                high: 160.0,
                drop: 0.04,
                tau: 0.11,
                drive: 2.5,
                click: 0.4,
                click_hz: 5000.0,
                click_tau: 0.003,
                seconds: 1.05,
            },
            seed,
        ),
        ("Foundry", "High Tom") => drum(
            &Drum {
                low: 140.0,
                high: 260.0,
                drop: 0.04,
                tau: 0.09,
                drive: 2.5,
                click: 0.4,
                click_hz: 5000.0,
                click_tau: 0.003,
                seconds: 0.8,
            },
            seed,
        ),
        ("Foundry", "Rim") => rim(
            &Rim {
                tones: [610.0, 2100.0],
                tau: 0.009,
                seconds: 0.12,
            },
            seed,
        ),
        ("Foundry", "Shaker") => shaker(
            &Shaker {
                band_hz: 8000.0,
                attack: 0.008,
                tau: 0.035,
                seconds: 0.3,
            },
            seed,
        ),
        ("Foundry", "Metal") => bell(&Bell {
            tones: &[319.0, 456.0, 677.0, 913.0, 1187.0, 1559.0],
            band_hz: 2800.0,
            q: 0.7,
            tau: 0.22,
            seconds: 1.8,
        }),
        ("Foundry", "Noise Hit") => noise_hit(9000.0, 300.0, 0.08, 0.11, 1.0, seed),
        _ => return None,
    };
    Some(match kit {
        "Dust" => lofi(samples),
        "Foundry" => {
            let peak = peak(&samples);
            samples.iter().map(|s| (2.0 * s / peak).tanh()).collect()
        }
        _ => samples,
    })
}

/// A factory pad's own noise seed: FNV-1a 32 of `<kit>/<pad>`, never 0 (xorshift's fixed point).
fn seed(kit: &str, pad: &str) -> u32 {
    let bytes = kit.bytes().chain([b'/']).chain(pad.bytes());
    bytes.fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    }) | 1
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// `seconds` of `voice(t)`.
fn render(seconds: f32, mut voice: impl FnMut(f32) -> f32) -> Vec<f32> {
    (0..(seconds * SR) as usize)
        .map(|n| voice(n as f32 / SR))
        .collect()
}

/// An exponential decay of time constant `tau` starting at `at`, silent before.
fn decay(t: f32, at: f32, tau: f32) -> f32 {
    if t < at { 0.0 } else { (-(t - at) / tau).exp() }
}

/// A biquad on the crate's filter law: low- and high-pass have an effective Q of about `resonance / 2`.
fn filter(response: Response, hz: f32, resonance: f32) -> Biquad {
    let mut biquad = Biquad::default();
    biquad.set_filter_response(response, hz, resonance, SR);
    biquad
}

/// Seeded white noise in [−1, 1) (xorshift32).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / 2_147_483_648.0 - 1.0
    }
}

/// The mean of square waves at `tones` Hz: a metallic, inharmonic buzz.
fn squares(tones: &[f32], t: f32) -> f32 {
    let sum: f32 = tones
        .iter()
        .map(|hz| if (hz * t).fract() < 0.5 { 1.0 } else { -1.0 })
        .sum();
    sum / tones.len() as f32
}

/// A sine whose pitch falls from `high` to `low` Hz with time constant `drop`, under a `tau` decay; it returns
/// the oscillator so each drum can add its own transient.
fn swept_sine(low: f32, high: f32, drop: f32, tau: f32) -> impl FnMut(f32) -> f32 {
    let mut phase = 0.0_f32;
    move |t| {
        let out = (TAU * phase).sin() * decay(t, 0.0, tau);
        phase = (phase + (low + (high - low) * decay(t, 0.0, drop)) / SR).fract();
        out
    }
}

/// Kicks and toms: a falling sine, `drive`n into `tanh` (0 = clean), and a low-passed noise click.
struct Drum {
    low: f32,
    high: f32,
    drop: f32,
    tau: f32,
    drive: f32,
    click: f32,
    click_hz: f32,
    click_tau: f32,
    seconds: f32,
}

const KICK: Drum = Drum {
    low: 52.0,
    high: 170.0,
    drop: 0.03,
    tau: 0.11,
    drive: 1.8,
    click: 0.25,
    click_hz: 3000.0,
    click_tau: 0.003,
    seconds: 0.9,
};

const TOM: Drum = Drum {
    low: 145.0,
    high: 235.0,
    drop: 0.06,
    tau: 0.085,
    drive: 0.0,
    click: 0.3,
    click_hz: 4000.0,
    click_tau: 0.004,
    seconds: 0.65,
};

/// A clean tom that settles on `hz`, rising half again at the strike.
fn tuned_tom(hz: f32) -> Drum {
    Drum {
        low: hz,
        high: 1.5 * hz,
        drop: 0.05,
        tau: 0.13,
        click: 0.25,
        click_hz: 3000.0,
        seconds: 1.0,
        ..TOM
    }
}

fn drum(drum: &Drum, seed: u32) -> Vec<f32> {
    let mut body = swept_sine(drum.low, drum.high, drum.drop, drum.tau);
    let mut noise = Noise(seed);
    let mut click = filter(Response::LowPass, drum.click_hz, 0.7);
    render(drum.seconds, |t| {
        let body = body(t);
        let body = if drum.drive > 0.0 {
            (drum.drive * body).tanh()
        } else {
            body
        };
        body + drum.click * click.process(noise.next()) * decay(t, 0.0, drum.click_tau)
    })
}

/// Two bent sines for the shell, high-passed then low-passed noise for the wires.
struct Snare {
    low: f32,
    high: f32,
    tone: f32,
    tone_tau: f32,
    hiss_hz: f32,
    dull_hz: f32,
    noise: f32,
    noise_tau: f32,
    seconds: f32,
}

const SNARE: Snare = Snare {
    low: 180.0,
    high: 290.0,
    tone: 0.5,
    tone_tau: 0.045,
    hiss_hz: 1800.0,
    dull_hz: 7000.0,
    noise: 0.7,
    noise_tau: 0.07,
    seconds: 0.5,
};

fn snare(snare: &Snare, seed: u32) -> Vec<f32> {
    let (mut low, mut high) = (0.0_f32, 0.0_f32);
    let mut noise = Noise(seed);
    let mut hiss = filter(Response::HighPass, snare.hiss_hz, 0.7);
    let mut dull = filter(Response::LowPass, snare.dull_hz, 0.7);
    render(snare.seconds, |t| {
        let bend = 1.0 + 0.15 * decay(t, 0.0, 0.01);
        low = (low + snare.low * bend / SR).fract();
        high = (high + snare.high * bend / SR).fract();
        let tone =
            (0.6 * (TAU * low).sin() + 0.4 * (TAU * high).sin()) * decay(t, 0.0, snare.tone_tau);
        snare.tone * tone
            + snare.noise
                * dull.process(hiss.process(noise.next()))
                * decay(t, 0.0, snare.noise_tau)
    })
}

/// Band-passed noise struck three times, `spread` apart, then a tail.
struct Clap {
    hz: f32,
    q: f32,
    spread: f32,
    tail_tau: f32,
    seconds: f32,
}

const CLAP: Clap = Clap {
    hz: 1100.0,
    q: 1.5,
    spread: 1.0,
    tail_tau: 0.07,
    seconds: 0.55,
};

fn clap(clap: &Clap, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut band = filter(Response::BandPass, clap.hz, clap.q);
    let strikes = [0.0, 0.011, 0.023].map(|at| at * clap.spread);
    let tail = 0.031 * clap.spread;
    render(clap.seconds, |t| {
        let bursts: f32 = strikes.iter().map(|&at| decay(t, at, 0.005)).sum();
        band.process(noise.next()) * (bursts + 0.6 * decay(t, tail, clap.tail_tau))
    })
}

/// Hats: the square stack (scaled by `tune`) and some noise, band- then high-passed.
struct Hat {
    tune: f32,
    noise: f32,
    band_hz: f32,
    hp_hz: f32,
    tau: f32,
    seconds: f32,
}

const CLOSED_HAT: Hat = Hat {
    tune: 1.0,
    noise: 0.3,
    band_hz: 9500.0,
    hp_hz: 6500.0,
    tau: 0.022,
    seconds: 0.18,
};

const OPEN_HAT: Hat = Hat {
    tau: 0.16,
    seconds: 1.2,
    ..CLOSED_HAT
};

fn hat(hat: &Hat, seed: u32) -> Vec<f32> {
    let tones = HAT_TONES.map(|hz| hz * hat.tune);
    let mut noise = Noise(seed);
    let mut band = filter(Response::BandPass, hat.band_hz, 1.0);
    let mut high = filter(Response::HighPass, hat.hp_hz, 0.7);
    render(hat.seconds, |t| {
        let source = squares(&tones, t) + hat.noise * noise.next();
        high.process(band.process(source)) * decay(t, 0.0, hat.tau)
    })
}

/// Crash, ride and cymbal: `metal` of the wide square stack over noise, high-passed, a splash then a wash.
struct Cymbal {
    tune: f32,
    metal: f32,
    hp_hz: f32,
    tau: f32,
    seconds: f32,
}

const CRASH: Cymbal = Cymbal {
    tune: 1.0,
    metal: 0.6,
    hp_hz: 4500.0,
    tau: 0.36,
    seconds: 2.6,
};

fn cymbal(cymbal: &Cymbal, seed: u32) -> Vec<f32> {
    let tones = CRASH_TONES.map(|hz| hz * cymbal.tune);
    let mut noise = Noise(seed);
    let mut high = filter(Response::HighPass, cymbal.hp_hz, 0.7);
    render(cymbal.seconds, |t| {
        let source = cymbal.metal * squares(&tones, t) + noise.next();
        high.process(source) * (0.5 * decay(t, 0.0, 0.03) + decay(t, 0.0, cymbal.tau))
    })
}

/// Perc, cowbell and metal: a band-limited stack of squares, a knock then a ring.
struct Bell {
    tones: &'static [f32],
    band_hz: f32,
    q: f32,
    tau: f32,
    seconds: f32,
}

const PERC: Bell = Bell {
    tones: &[587.0, 868.0],
    band_hz: 2400.0,
    q: 0.8,
    tau: 0.06,
    seconds: 0.45,
};

fn bell(bell: &Bell) -> Vec<f32> {
    let mut band = filter(Response::BandPass, bell.band_hz, bell.q);
    render(bell.seconds, |t| {
        band.process(squares(bell.tones, t))
            * (0.6 * decay(t, 0.0, 0.012) + 0.4 * decay(t, 0.0, bell.tau))
    })
}

/// A rimshot: two sines ringing briefly, driven, over a 2 ms high-passed snap.
struct Rim {
    tones: [f32; 2],
    tau: f32,
    seconds: f32,
}

fn rim(rim: &Rim, seed: u32) -> Vec<f32> {
    let mut phases = [0.0_f32; 2];
    let mut noise = Noise(seed);
    let mut snap = filter(Response::HighPass, 2500.0, 0.7);
    render(rim.seconds, |t| {
        let mut ring = 0.0;
        for (phase, hz) in phases.iter_mut().zip(rim.tones) {
            ring += 0.5 * (TAU * *phase).sin();
            *phase = (*phase + hz / SR).fract();
        }
        (2.0 * ring * decay(t, 0.0, rim.tau)).tanh()
            + 0.5 * snap.process(noise.next()) * decay(t, 0.0, 0.002)
    })
}

/// Band-passed noise that swells over `attack` and dies away.
struct Shaker {
    band_hz: f32,
    attack: f32,
    tau: f32,
    seconds: f32,
}

fn shaker(shaker: &Shaker, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut band = filter(Response::BandPass, shaker.band_hz, 1.2);
    let mut high = filter(Response::HighPass, 4000.0, 0.7);
    render(shaker.seconds, |t| {
        let swell = (t / shaker.attack).min(1.0);
        high.process(band.process(noise.next())) * swell * decay(t, 0.0, shaker.tau)
    })
}

/// Noise through a resonant low-pass that sweeps from `from` down to `to` Hz (time constant `sweep`), driven.
fn noise_hit(from: f32, to: f32, sweep: f32, tau: f32, seconds: f32, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut low = Biquad::default();
    render(seconds, |t| {
        low.set_filter_response(
            Response::LowPass,
            to + (from - to) * decay(t, 0.0, sweep),
            4.0,
            SR,
        );
        (1.5 * low.process(noise.next())).tanh() * decay(t, 0.0, tau)
    })
}

/// Vinyl crackle: pops of seeded size at about `rate` a second (the first at once), each a short band-passed ring,
/// over a faint hiss, all under a `tau` decay.
fn crackle(rate: f32, tau: f32, seconds: f32, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut ring = filter(Response::BandPass, 2800.0, 0.9);
    let mut first = true;
    render(seconds, |t| {
        let roll = 0.5 + 0.5 * noise.next();
        let size = noise.next();
        let pop = if first || roll < rate / SR {
            first = false;
            size.signum() * (0.3 + 0.7 * size * size)
        } else {
            0.0
        };
        (8.0 * ring.process(pop) + 0.02 * noise.next()) * decay(t, 0.0, tau)
    })
}

/// Dust's chain: ½-rate hold, 12-bit steps of the peak, two ≈7 kHz low-passes.
fn lofi(mut samples: Vec<f32>) -> Vec<f32> {
    let peak = peak(&samples);
    let mut lows = [0; 2].map(|_| filter(Response::LowPass, 7000.0, 1.4));
    let mut held = 0.0;
    for (i, s) in samples.iter_mut().enumerate() {
        if i % 2 == 0 {
            held = (*s / peak * 2047.0).round() / 2047.0;
        }
        *s = lows.iter_mut().fold(held, |x, low| low.process(x));
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full-scale first frame, then `level` (12-bit steps are of that peak).
    fn peak_then(level: f32) -> Vec<f32> {
        let mut samples = vec![level; 4000];
        samples[0] = 1.0;
        samples
    }

    #[test]
    fn lofi_holds_every_other_frame() {
        let mut noise = Noise(7);
        let mut x = peak_then(0.0);
        x[1..].iter_mut().for_each(|s| *s = 0.5 * noise.next());
        let mut y = x.clone();
        y.iter_mut().skip(1).step_by(2).for_each(|s| *s = -*s);
        assert_eq!(lofi(x), lofi(y), "odd frames are never heard");

        // A 16 kHz tone held at 22.05 kHz folds to 6.05 kHz.
        let tone = render(0.5, |t| (TAU * 16_000.0 * t).sin());
        let out = lofi(tone);
        let rising = (1..out.len())
            .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
            .count();
        let hz = rising as f32 / 0.5;
        assert!((hz - 6050.0).abs() < 100.0, "comes out at {hz} Hz");
    }

    #[test]
    fn lofi_rounds_to_12_bit_steps() {
        let tail = |level| *lofi(peak_then(level)).last().unwrap();
        assert_eq!(tail(1e-4), 0.0, "under half a step is silence");
        assert!(tail(3e-4) > 0.0);
        assert_eq!(tail(3e-4), tail(4e-4), "both round to one step");
    }

    #[test]
    fn foundry_pads_are_soft_clipped() {
        let (_, pads) = KITS.iter().find(|(key, _)| *key == "Foundry").unwrap();
        for pad in *pads {
            let samples = kit_pad("Foundry", pad).unwrap();
            assert_eq!(peak(&samples), 2.0_f32.tanh(), "{pad}");
        }
    }

    #[test]
    fn crackle_pops_about_30_times_a_second() {
        let (tau, seconds) = (0.3, 1.6);
        let samples = crackle(30.0, tau, seconds, seed("Dust", "Crackle"));
        // Undo the decay; a pop rings above 0.38, the hiss stays under 0.02. Pops 2 ms apart count once.
        let mut last = None;
        let mut pops = 0;
        for (n, s) in samples.iter().enumerate() {
            if (s / decay(n as f32 / SR, 0.0, tau)).abs() > 0.1 {
                if last.is_none_or(|at| n - at > RATE as usize / 500) {
                    pops += 1;
                }
                last = Some(n);
            }
        }
        let expected = 30.0 * seconds;
        assert!(
            (0.7 * expected..=1.3 * expected).contains(&(pops as f32)),
            "{pops} pops in {seconds} s"
        );
    }
}
