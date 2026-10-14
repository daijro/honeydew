//! GMSynth playing a user's original waveshape in place of a designed one. Its own test binary: the
//! shapes are per process.

use std::f64::consts::TAU;

use honeydew_audio::ProcessContext;
use honeydew_audio::gm_waveshapes::{PARTIALS, SAW, SLOTS, Spectrum};
use honeydew_audio::gmsynth::{GmSynth, WAVE_A, WAVE_B, install_shape, prepare_setting};
use honeydew_audio::instrument::Instrument;

const RATE: f32 = 48_000.0;

/// A sine in the `.spectra` layout: partial 1 at 0.5.
fn sine() -> Spectrum {
    let mut mag = [0.0; PARTIALS];
    mag[0] = 0.5;
    Spectrum {
        mag,
        phase: [0.0; PARTIALS],
    }
}

/// The amplitude of the `freq` Hz component (Goertzel, Hann window).
fn amplitude(signal: &[f32], freq: f64) -> f64 {
    let n = signal.len() as f64;
    let coefficient = 2.0 * (TAU * freq / f64::from(RATE)).cos();
    let (mut s1, mut s2, mut weight) = (0.0, 0.0, 0.0);
    for (i, &x) in signal.iter().enumerate() {
        let w = 0.5 - 0.5 * (TAU * i as f64 / n).cos();
        weight += w;
        let s0 = w * f64::from(x) + coefficient * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    2.0 * (s1 * s1 + s2 * s2 - coefficient * s1 * s2).max(0.0).sqrt() / weight
}

/// Both oscillators on `slot`, key 69 (440 Hz), 0.5 s.
fn play(slot: usize) -> Vec<f32> {
    let mut synth = GmSynth::new(RATE);
    for key in [WAVE_A, WAVE_B] {
        prepare_setting(key, slot as f32);
        synth.set_setting(key, slot as f32);
    }
    synth.reset();
    synth.note_on(69, 1.0, 0.5);
    let (mut left, mut right) = (vec![0.0; 24_000], vec![0.0; 24_000]);
    let context = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };
    synth.process(&mut left, &mut right, context);
    left
}

#[test]
fn an_installed_shape_replaces_the_designed_one_once_and_only_before_it_is_built() {
    // Slot 0 (Beast) is designed with many partials; as the user's sine it has one.
    assert!(install_shape(0, sine()));
    assert!(!install_shape(0, sine()), "once per slot");
    let tone = play(0);
    let (fundamental, second) = (amplitude(&tone, 440.0), amplitude(&tone, 880.0));
    assert!(fundamental > 0.01, "{fundamental}");
    assert!(second < fundamental * 1e-3, "{second} / {fundamental}");
    // A slot already built keeps its tables; past the menu is no slot.
    play(SAW);
    assert!(!install_shape(SAW, sine()));
    assert!(!install_shape(SLOTS, sine()));
}
