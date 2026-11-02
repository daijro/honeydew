//! Transition and texture models: risers and downlifters, Shepard rises, impacts, whooshes, vinyl crackle and tape
//! stops. All stereo, with each side's noise its own stretch of the seeded stream.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use super::drums::{drive, tail};
use super::{Noise, Osc, P, SR, Svf, secs};

/// `length` s of noise, mixed with three detuned saws gliding from `pitch_from` to `pitch_to` Hz at level `saw`, into a
/// resonant (`q`) low-pass swept exponentially from `from` to `to` Hz. `up` 1 swells to its peak at `length` and then
/// rings out over 0.4 s (a riser); 0 falls away from the start (a downlifter).
pub const RISER: &[(&str, f32)] = &[
    ("length", 4.0),
    ("from", 300.0),
    ("to", 9000.0),
    ("q", 4.0),
    ("saw", 0.0),
    ("pitch_from", 110.0),
    ("pitch_to", 880.0),
    ("up", 1.0),
];
pub const DOWNLIFTER: &[(&str, f32)] = &[
    ("length", 3.0),
    ("from", 8000.0),
    ("to", 200.0),
    ("q", 3.0),
    ("saw", 0.0),
    ("pitch_from", 880.0),
    ("pitch_to", 55.0),
    ("up", 0.0),
];

pub fn sweep(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let (swell, up) = (secs(p["length"]), p["up"] > 0.5);
    let len = swell + if up { secs(0.4) } else { 0 };
    let edge = secs(0.03) as f32;
    let side = |detune: [f32; 3], noise: &mut Noise| {
        let (mut lp, mut saws) = (
            Svf::default(),
            [Osc::default(), Osc::default(), Osc::default()],
        );
        (0..len)
            .map(|n| {
                let x = (n as f32 / swell as f32).min(1.0);
                let hz = p["from"] * (p["to"] / p["from"]).powf(x);
                let pitch = p["pitch_from"] * (p["pitch_to"] / p["pitch_from"]).powf(x);
                let saw: f32 = saws
                    .iter_mut()
                    .zip(detune)
                    .map(|(s, d)| s.saw(pitch * d))
                    .sum();
                let src = (1.0 - p["saw"]) * noise.next() + p["saw"] * saw / 3.0;
                let amp = if up {
                    x * x * (-(n.saturating_sub(swell) as f32) / (0.05 * SR)).exp()
                } else {
                    (1.0 - x) * (1.0 - x) * (n as f32 / edge).min(1.0)
                };
                lp.tick(src, hz, p["q"]).0 * amp
            })
            .collect()
    };
    vec![
        side([0.993, 1.0, 1.006], noise),
        side([1.007, 1.0, 0.995], noise),
    ]
}

/// Eight sines an octave apart climbing `rate` octaves a second (falling when negative) from `base` Hz, each faded in
/// and out over its eight-octave span so the climb never ends, for `length` s. The right side is a hair sharp.
pub const SHEPARD: &[(&str, f32)] = &[("length", 6.0), ("rate", 0.5), ("base", 40.0)];

pub fn shepard(p: &P, _: &mut Noise) -> Vec<Vec<f32>> {
    let len = secs(p["length"]);
    let side = |detune: f32| {
        let mut phases = [0.0_f32; 8];
        (0..len)
            .map(|n| {
                let t = n as f32 / SR;
                let swell = (t / 0.5).min(1.0) * ((p["length"] - t) / 0.3).min(1.0).powi(3);
                let tones: f32 = (phases.iter_mut().enumerate())
                    .map(|(k, ph)| {
                        let pos = (k as f32 + p["rate"] * t).rem_euclid(8.0);
                        *ph = (*ph + p["base"] * 2f32.powf(pos) * detune / SR).fract();
                        (0.5 - 0.5 * (TAU * pos / 8.0).cos()) * (TAU * *ph).sin()
                    })
                    .sum();
                tones * swell
            })
            .collect()
    };
    vec![side(1.0), side(1.003)]
}

/// A boom at `tune` Hz falling from `bend` × `tune`, decaying with `decay`, driven; over it `noise` through a low-pass
/// falling from `tone` Hz, until the boom has died away.
pub const IMPACT: &[(&str, f32)] = &[
    ("tune", 45.0),
    ("bend", 4.0),
    ("decay", 0.6),
    ("noise", 0.6),
    ("tone", 6000.0),
];

pub fn impact(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let len = secs(p["decay"] * tail(2.0));
    let mut phase = 0.0_f32;
    let boom: Vec<f32> = (0..len)
        .map(|n| {
            let t = n as f32 / SR;
            phase =
                (phase + p["tune"] * (1.0 + (p["bend"] - 1.0) * (-t / 0.08).exp()) / SR).fract();
            drive((TAU * phase).sin() * (-t / p["decay"]).exp(), 2.0)
        })
        .collect();
    let side = |noise: &mut Noise| {
        let mut lp = Svf::default();
        (boom.iter().enumerate())
            .map(|(n, b)| {
                let t = n as f32 / SR;
                let hz = 200.0 + p["tone"] * (-t / 0.15).exp();
                b + p["noise"] * lp.tick(noise.next(), hz, 0.8).0 * (-t / (0.5 * p["decay"])).exp()
            })
            .collect()
    };
    vec![side(noise), side(noise)]
}

/// `length` s of noise through a band-pass (`q`) that rises from `lo` to `hi` Hz and back, swelling and fading with
/// it, panned left to right.
pub const WHOOSH: &[(&str, f32)] = &[("length", 1.5), ("lo", 300.0), ("hi", 4000.0), ("q", 2.0)];

pub fn whoosh(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let len = secs(p["length"]);
    let mut bp = Svf::default();
    let mono: Vec<(f32, f32)> = (0..len)
        .map(|n| {
            let x = n as f32 / len as f32;
            let s = (PI * x).sin().powi(2);
            (
                bp.tick(noise.next(), p["lo"] + (p["hi"] - p["lo"]) * s, p["q"])
                    .1
                    * s,
                x,
            )
        })
        .collect();
    let pan = |f: fn(f32) -> f32| mono.iter().map(|&(y, x)| y * f(FRAC_PI_2 * x)).collect();
    vec![pan(f32::cos), pan(f32::sin)]
}

/// `length` s of vinyl: `density` pops a second, each a tiny random-sized click, over low-passed `hiss`, high-passed at
/// 150 Hz, fading out over its last 200 ms.
pub const CRACKLE: &[(&str, f32)] = &[("length", 4.0), ("density", 18.0), ("hiss", 0.08)];

pub fn crackle(p: &P, noise: &mut Noise) -> Vec<Vec<f32>> {
    let len = secs(p["length"]);
    let ring = (-1.0 / (0.0007 * SR)).exp();
    let side = |noise: &mut Noise| {
        let (mut pop, mut lp, mut hp) = (0.0_f32, Svf::default(), Svf::default());
        (0..len)
            .map(|n| {
                if (noise.next() + 1.0) * 0.5 < p["density"] / SR {
                    pop += noise.next().powi(3);
                }
                pop *= ring;
                let hiss = p["hiss"] * lp.tick(noise.next(), 3500.0, 0.7).0;
                let fade = ((len - n) as f32 / secs(0.2) as f32).min(1.0).powi(3);
                hp.tick(pop + hiss, 150.0, 0.7).2 * fade
            })
            .collect()
    };
    vec![side(noise), side(noise)]
}

/// A saw chord on `tune` Hz (root, fifth, octave) that plays `hold` s and then slows to a stop over `stop` s, its
/// pitch, brightness and level falling with the speed.
pub const TAPESTOP: &[(&str, f32)] = &[("tune", 110.0), ("hold", 0.6), ("stop", 0.8)];

pub fn tapestop(p: &P, _: &mut Noise) -> Vec<Vec<f32>> {
    let len = secs(p["hold"] + p["stop"]);
    let side = |detune: f32| {
        let (mut lp, mut saws) = (
            Svf::default(),
            [Osc::default(), Osc::default(), Osc::default()],
        );
        (0..len)
            .map(|n| {
                let t = n as f32 / SR;
                let speed = (1.0 - (t - p["hold"]).max(0.0) / p["stop"])
                    .max(0.0)
                    .powf(1.5);
                let chord: f32 = (saws.iter_mut().zip([1.0, 1.5, 2.0]))
                    .map(|(s, r)| s.saw(p["tune"] * r * detune * speed))
                    .sum();
                lp.tick(chord / 3.0, 300.0 + 5000.0 * speed, 0.9).0 * speed.powf(1.5)
            })
            .collect()
    };
    vec![side(0.997), side(1.003)]
}
