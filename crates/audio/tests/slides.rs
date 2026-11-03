//! Slide offsets per instrument: MiniSynth, GMSynth, SuperSaw and
//! 3x Osc add the slide's pitch, pan and velocity offsets to every voice, and a newly launched voice starts without
//! them; the Transistor Bass ignores slides. (The DW Sampler and SoundFont Player are tested with their fixtures.)

use honeydew_audio::ProcessContext;
use honeydew_audio::gmsynth::GmSynth;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::minisynth::MiniSynth;
use honeydew_audio::supersaw::SuperSaw;
use honeydew_audio::three_osc::ThreeOsc;
use honeydew_audio::transistor_bass::TransistorBass;

const RATE: f32 = 48_000.0;

fn synths() -> Vec<(&'static str, Box<dyn Instrument>)> {
    vec![
        ("MiniSynth", Box::new(MiniSynth::new(RATE).unwrap())),
        ("GMSynth", Box::new(GmSynth::new(RATE))),
        ("SuperSaw", Box::new(SuperSaw::new(RATE))),
        ("3x Osc", Box::new(ThreeOsc::new(RATE, 4))),
    ]
}

/// `frames` frames of `synth`, left and right.
fn render(synth: &mut dyn Instrument, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let context = ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat: 0.0,
        playing: false,
    };
    for (l, r) in left.chunks_mut(256).zip(right.chunks_mut(256)) {
        synth.process(l, r, context);
    }
    (left, right)
}

fn energy(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s * s).sum()
}

/// Sign changes per frame.
fn crossings(samples: &[f32]) -> f32 {
    let changes = samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    changes as f32 / samples.len() as f32
}

/// Key 57 held for 0.1 s, then `slide` applied, then `before` more key-downs; the crossing rate and the left and
/// right energy of the next 0.2 s, after 0.1 s to settle.
fn after_slide(
    synth: &mut dyn Instrument,
    slide: [f32; 3],
    before: impl FnOnce(&mut dyn Instrument),
) -> (f32, f32, f32) {
    synth.note_on(57, 1.0, 0.5);
    render(synth, 4800);
    synth.set_slide(slide[0], slide[1], slide[2]);
    before(synth);
    render(synth, 4800);
    let (left, right) = render(synth, 9600);
    (crossings(&left), energy(&left), energy(&right))
}

#[test]
fn slides_add_pitch_pan_and_velocity_to_every_sounding_voice() {
    for ((name, mut plain), (_, mut slid)) in synths().into_iter().zip(synths()) {
        let (rate, left, right) = after_slide(plain.as_mut(), [0.0; 3], |_| {});
        assert!(left > 0.01 && right > 0.01, "{name} sounds");
        let (octave, ..) = after_slide(slid.as_mut(), [12.0, 0.0, 0.0], |_| {});
        assert!(
            (octave / rate - 2.0).abs() < 0.1,
            "{name}: {octave} / {rate}"
        );
    }
    for (name, mut synth) in synths() {
        // p = clamp(pan + 1, 0, 1) = 1: hard right.
        let (_, left, right) = after_slide(synth.as_mut(), [0.0, 1.0, 0.0], |_| {});
        assert!(left < 1e-4 * right, "{name}: left {left}, right {right}");
    }
    for (name, mut synth) in synths() {
        // Velocity + slide velocity ≤ 0: no level.
        let (_, left, right) = after_slide(synth.as_mut(), [0.0, 0.0, -1.0], |_| {});
        assert!(left + right < 1e-6, "{name}: {left} {right}");
    }
}

#[test]
fn a_newly_launched_voice_starts_without_the_offsets() {
    for ((name, mut plain), (_, mut slid)) in synths().into_iter().zip(synths()) {
        let (rate, ..) = after_slide(plain.as_mut(), [0.0; 3], |_| {});
        // The slide silences key 57 (velocity − 1) and shifts it; key 45 then starts on a voice of its own, an octave
        // below 57 and unshifted.
        let (lower, left, right) = after_slide(slid.as_mut(), [12.0, 0.0, -1.0], |synth| {
            synth.note_on(45, 1.0, 0.5);
        });
        assert!(left > 0.01 && right > 0.01, "{name} sounds");
        assert!(
            (lower / rate - 0.5).abs() < 0.05,
            "{name}: {lower} / {rate}"
        );
    }
}

/// A slide after a note-on on the same frame reaches that note's voice, also where the voice starts only at the next
/// control tick (3x Osc), so a 1-tick slide is not lost; a key pressed after the slide, in the same
/// tick gap, still starts without it.
#[test]
fn a_slide_right_after_a_note_on_reaches_its_voice() {
    let all = synths().into_iter().zip(synths()).zip(synths());
    for (((name, mut plain), (_, mut slid)), (_, mut late)) in all {
        plain.note_on(57, 1.0, 0.5);
        let (rate, ..) = settled(plain.as_mut());
        slid.note_on(57, 1.0, 0.5);
        slid.set_slide(12.0, 0.0, 0.0);
        let (octave, ..) = settled(slid.as_mut());
        assert!(
            (octave / rate - 2.0).abs() < 0.1,
            "{name}: {octave} / {rate}"
        );
        // The slide silences key 57 and shifts it; key 45, pressed after it, sounds an octave below 57, unshifted.
        late.note_on(57, 1.0, 0.5);
        late.set_slide(12.0, 0.0, -1.0);
        late.note_on(45, 1.0, 0.5);
        let (lower, left, right) = settled(late.as_mut());
        assert!(left > 0.01 && right > 0.01, "{name} sounds");
        assert!(
            (lower / rate - 0.5).abs() < 0.05,
            "{name}: {lower} / {rate}"
        );
    }
}

/// MiniSynth zeroes the offsets only when the voice it takes was idle: MONO retriggers voice 1, which
/// keeps them.
#[test]
fn minisynth_keeps_the_offsets_on_a_voice_that_was_sounding() {
    let mono = || {
        let mut synth = MiniSynth::new(RATE).unwrap();
        synth.set_parameter(1, 0.5).unwrap();
        synth
    };
    let (rate, ..) = after_slide(&mut mono(), [0.0; 3], |_| {});
    let (retriggered, ..) = after_slide(&mut mono(), [12.0, 0.0, 0.0], |synth| {
        synth.note_on(57, 1.0, 0.5)
    });
    assert!(
        (retriggered / rate - 2.0).abs() < 0.1,
        "{retriggered} / {rate}"
    );
}

#[test]
fn the_transistor_bass_ignores_slides() {
    let (mut plain, mut slid) = (TransistorBass::new(RATE), TransistorBass::new(RATE));
    let expected = after_slide(&mut plain, [0.0; 3], |_| {});
    assert!(expected.1 > 0.01);
    assert_eq!(after_slide(&mut slid, [12.0, 0.5, -1.0], |_| {}), expected);
}

/// The crossing rate of `synth`'s left side over 0.2 s, after 0.1 s to settle, and its left and right energy.
fn settled(synth: &mut dyn Instrument) -> (f32, f32, f32) {
    render(synth, 4800);
    let (left, right) = render(synth, 9600);
    (crossings(&left), energy(&left), energy(&right))
}

/// The crossing rate of `key` alone on a fresh 3x Osc.
fn three_osc_rate(key: u8) -> f32 {
    let mut synth = ThreeOsc::new(RATE, 4);
    synth.note_on(key, 1.0, 0.5);
    settled(&mut synth).0
}

/// 3x Osc: a reset or an all-notes-off forgets a slide that came while keys were queued, so a key queued afterwards
/// starts without it.
#[test]
fn three_osc_reset_and_all_notes_off_forget_a_queued_slide() {
    let rate = three_osc_rate(57);
    for name in ["reset", "all notes off"] {
        let mut synth = ThreeOsc::new(RATE, 4);
        synth.note_on(60, 1.0, 0.5);
        synth.set_slide(12.0, 0.0, 0.0);
        if name == "reset" {
            synth.reset();
        } else {
            synth.all_notes_off();
        }
        synth.note_on(57, 1.0, 0.5);
        let (unshifted, ..) = settled(&mut synth);
        assert!(
            (unshifted / rate - 1.0).abs() < 0.05,
            "{name}: {unshifted} / {rate}"
        );
    }
}

/// 3x Osc: with the key queue full, an off that cancels a queued on before a slide moves the slide's place with it, so
/// a key queued after the slide still starts without it.
#[test]
fn three_osc_a_cancelled_queued_key_keeps_the_slide_in_place() {
    let rate = three_osc_rate(64);
    let mut synth = ThreeOsc::new(RATE, 4);
    synth.note_on(57, 1.0, 0.5);
    synth.note_on(60, 1.0, 0.5);
    // Silences and shifts 60.
    synth.set_slide(12.0, 0.0, -1.0);
    synth.note_on(64, 1.0, 0.5);
    for _ in 0..300 {
        synth.note_off(100);
    }
    synth.note_off(57);
    let (unshifted, left, right) = settled(&mut synth);
    assert!(left > 0.01 && right > 0.01, "64 sounds");
    assert!(
        (unshifted / rate - 1.0).abs() < 0.05,
        "{unshifted} / {rate}"
    );
}
