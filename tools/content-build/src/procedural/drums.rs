//! Drum models: a pitched membrane (kicks and toms), snare, clap, six detuned squares into a band-pass (hats and
//! cymbals), cowbell, clave and rim. All mono.

use std::f32::consts::TAU;

use super::{Noise, Osc, P, SR, Svf, env, secs};

/// `tune` Hz after a pitch drop from `bend` × `tune` (time constant `sweep` s), amplitude held `hold` s then decaying
/// with `decay` s; a high-passed noise `click`, a band of `noise` around 2 × `tune`, a `sub` an octave down and a
/// `drive` (tanh) over the lot.
pub const KICK: &[(&str, f32)] = &[
    ("tune", 55.0),
    ("bend", 4.0),
    ("sweep", 0.03),
    ("decay", 0.3),
    ("hold", 0.0),
    ("click", 0.0),
    ("noise", 0.0),
    ("sub", 0.0),
    ("drive", 0.0),
];
pub const TOM: &[(&str, f32)] = &[
    ("tune", 120.0),
    ("bend", 1.6),
    ("sweep", 0.05),
    ("decay", 0.22),
    ("hold", 0.0),
    ("click", 0.1),
    ("noise", 0.15),
    ("sub", 0.0),
    ("drive", 0.0),
];

pub fn membrane(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let (tune, decay, hold) = (p["tune"], p["decay"], p["hold"]);
    let (mut body, mut sub) = (0.0_f32, 0.0_f32);
    let (mut hp, mut bp) = (Svf::default(), Svf::default());
    let tau = if p["sub"] > 0.0 { 1.5 * decay } else { decay };
    let len = secs(hold + tau * tail(p["drive"]) + 0.01);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let f = tune * (1.0 + (p["bend"] - 1.0) * (-t / p["sweep"]).exp());
        body = (body + f / SR).fract();
        sub = (sub + 0.5 * f / SR).fract();
        let w = noise.next();
        let x = (TAU * body).sin() * env(t, hold, decay)
            + p["sub"] * (TAU * sub).sin() * env(t, hold, 1.5 * decay)
            + p["click"] * hp.tick(w, 4000.0, 0.7).2 * (-t / 0.004).exp()
            + p["noise"] * bp.tick(w, 2.0 * tune, 1.0).1 * (-t / (0.3 * decay)).exp();
        drive(x, p["drive"])
    });
    vec![out.collect()]
}

/// Two sines (`tune`, 1.62 × `tune`) with a short pitch drop, decaying with `decay`, and noise band-passed at `tone`
/// at level `snappy`, decaying with `rattle`.
pub const SNARE: &[(&str, f32)] = &[
    ("tune", 185.0),
    ("decay", 0.07),
    ("snappy", 0.7),
    ("rattle", 0.14),
    ("tone", 5000.0),
    ("drive", 0.0),
];

pub fn snare(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let (mut a, mut b, mut bp) = (0.0_f32, 0.0_f32, Svf::default());
    let len = secs(p["decay"].max(p["rattle"]) * tail(p["drive"]));
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let f = p["tune"] * (1.0 + 0.4 * (-t / 0.01).exp());
        a = (a + f / SR).fract();
        b = (b + 1.62 * f / SR).fract();
        let body = (0.7 * (TAU * a).sin() + 0.4 * (TAU * b).sin()) * (-t / p["decay"]).exp();
        let rattle = bp.tick(noise.next(), p["tone"], 0.5).1 * (-t / p["rattle"]).exp();
        drive(body + p["snappy"] * 2.0 * rattle, p["drive"])
    });
    vec![out.collect()]
}

/// `bursts` noise bursts `spread` s apart, then a tail decaying with `decay`, band-passed at `tone` with `q`.
pub const CLAP: &[(&str, f32)] = &[
    ("tone", 1100.0),
    ("q", 1.4),
    ("decay", 0.16),
    ("spread", 0.011),
    ("bursts", 4.0),
];

pub fn clap(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let bursts = p["bursts"].max(1.0) as usize;
    let last = (bursts - 1) as f32 * p["spread"];
    let mut bp = Svf::default();
    let len = secs(last + 7.0 * p["decay"]);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let hits: f32 = (0..bursts)
            .map(|k| t - k as f32 * p["spread"])
            .filter(|&dt| dt >= 0.0)
            .map(|dt| (-dt / 0.0035).exp())
            .sum();
        let tail = if t >= last {
            0.5 * (-(t - last) / p["decay"]).exp()
        } else {
            0.0
        };
        bp.tick(noise.next(), p["tone"], p["q"]).1 * (hits + tail)
    });
    vec![out.collect()]
}

/// Six detuned squares from `tune` plus `noise`, band-passed at `tone` with `q`, high-passed at 0.6 × `tone`, decaying
/// with `decay`.
pub const HAT: &[(&str, f32)] = &[
    ("tune", 320.0),
    ("tone", 8500.0),
    ("q", 1.0),
    ("decay", 0.05),
    ("noise", 0.15),
];
pub const CYMBAL: &[(&str, f32)] = &[
    ("tune", 250.0),
    ("tone", 6000.0),
    ("q", 0.8),
    ("decay", 0.9),
    ("noise", 0.3),
];
/// Our inharmonic square ratios.
const METAL: [f32; 6] = [1.0, 1.342, 1.594, 1.877, 2.391, 2.773];

pub fn metal(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    // Seeded start phases, so the squares do not all step together.
    let mut squares: [Osc; 6] = std::array::from_fn(|_| Osc(0.5 + 0.5 * noise.next()));
    let (mut bp, mut hp) = (Svf::default(), Svf::default());
    let len = secs(7.0 * p["decay"]);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let sum: f32 = (squares.iter_mut().zip(METAL))
            .map(|(o, r)| o.square(p["tune"] * r))
            .sum();
        let x = sum / 6.0 + p["noise"] * noise.next();
        let band = bp.tick(x, p["tone"], p["q"]).1;
        hp.tick(band, 0.6 * p["tone"], 0.7).2 * (-t / p["decay"]).exp()
    });
    vec![out.collect()]
}

/// Two squares (`tune`, 1.48 × `tune`) band-passed at 2.2 × `tune`: a sharp strike over a `decay` ring.
pub const COWBELL: &[(&str, f32)] = &[("tune", 560.0), ("decay", 0.15)];

pub fn cowbell(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let (mut a, mut b) = (Osc(0.5 + 0.5 * noise.next()), Osc(0.5 + 0.5 * noise.next()));
    let mut bp = Svf::default();
    let len = secs(7.0 * p["decay"]);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let x = bp
            .tick(
                a.square(p["tune"]) + b.square(1.48 * p["tune"]),
                2.2 * p["tune"],
                1.2,
            )
            .1;
        x * (0.6 * (-t / 0.01).exp() + 0.4 * (-t / p["decay"]).exp())
    });
    vec![out.collect()]
}

/// A short sine ping at `tune` with a faint partial at 2.1 × `tune`.
pub const CLAVE: &[(&str, f32)] = &[("tune", 2500.0), ("decay", 0.025)];

pub fn clave(p: &P, _: &mut Noise) -> Vec<Vec<f32>> {
    let (mut a, mut b) = (0.0_f32, 0.0_f32);
    let len = secs(7.0 * p["decay"]);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        let f = p["tune"] * (1.0 + 0.05 * (-t / 0.002).exp());
        a = (a + f / SR).fract();
        b = (b + 2.1 * f / SR).fract();
        ((TAU * a).sin() + 0.2 * (TAU * b).sin()) * (-t / p["decay"]).exp()
    });
    vec![out.collect()]
}

/// Two partials (`tune`, 2.37 × `tune`) and a noise `click`, high-passed at 300 Hz.
pub const RIM: &[(&str, f32)] = &[("tune", 1700.0), ("decay", 0.018), ("click", 0.5)];

pub fn rim(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let (mut a, mut b, mut hp) = (0.0_f32, 0.0_f32, Svf::default());
    let len = secs(7.0 * p["decay"] + 0.005);
    let out = (0..len).map(|n| {
        let t = n as f32 / SR;
        a = (a + p["tune"] / SR).fract();
        b = (b + 2.37 * p["tune"] / SR).fract();
        let x = ((TAU * a).sin() + 0.5 * (TAU * b).sin()) * (-t / p["decay"]).exp()
            + p["click"] * noise.next() * (-t / 0.002).exp();
        hp.tick(x, 300.0, 0.7).2
    });
    vec![out.collect()]
}

/// Time constants until a decay driven by `d` is under −60 dB: 7, plus the drive's small-signal gain, ln(d / tanh d).
pub fn tail(d: f32) -> f32 {
    7.0 + if d > 0.0 { (d / d.tanh()).ln() } else { 0.0 }
}

/// `tanh(d·x) / tanh(d)`, which keeps ±1 at ±1; `x` itself for `d` ≤ 0.
pub fn drive(x: f32, d: f32) -> f32 {
    if d > 0.0 {
        (d * x).tanh() / d.tanh()
    } else {
        x
    }
}
