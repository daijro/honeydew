use honeydew_audio::ProcessContext;
use honeydew_audio::instrument::Instrument;
use honeydew_audio::minisynth::{MiniSynth, MiniSynthError, PARAMETERS};
use honeydew_audio::voice::{
    ARP_RATE_BEATS, GMSYNTH_RATE_BEATS, LFO_SYNC_BEATS, SUPERSAW_DELAY_BEATS, SUPERSAW_LFO_BEATS,
    lfo_attack_increment, lfo_rate_hz, tempo_step,
};

fn render(synth: &mut MiniSynth, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    synth.process(&mut left, &mut right).unwrap();
    (left, right)
}

#[test]
fn default_note_generates_stereo_audio_and_releases() {
    let mut synth = MiniSynth::new(48_000.0).unwrap();
    assert_eq!(PARAMETERS.len(), 44);
    assert_eq!(synth.parameter(9), Some(1.0));
    assert_eq!(render(&mut synth, 128).0, [0.0; 128]);

    synth.note_on(69, 1.0);
    let (left, right) = render(&mut synth, 1024);
    assert_eq!(left, right);
    assert!(left.iter().any(|sample| sample.abs() > 0.01));
    assert!(left.iter().all(|sample| sample.is_finite()));

    synth.note_off(69);
    let (left, _) = render(&mut synth, 48_000);
    assert!(left[47_000..].iter().all(|sample| sample.abs() < 0.001));
}

#[test]
fn simultaneous_notes_survive_independent_note_off() {
    let mut synth = MiniSynth::new(48_000.0).unwrap();
    synth.set_parameter(25, 0.0).unwrap();
    synth.note_on(60, 1.0);
    synth.note_on(64, 1.0);
    render(&mut synth, 256);
    synth.note_off(60);
    let (left, _) = render(&mut synth, 2048);
    assert!(left[1024..].iter().any(|sample| sample.abs() > 0.01));
    synth.note_off(64);
    let (left, _) = render(&mut synth, 2048);
    assert!(left[1024..].iter().all(|sample| sample.abs() < 0.001));
}

#[test]
fn parameter_changes_and_invalid_input_are_handled() {
    let mut synth = MiniSynth::new(48_000.0).unwrap();
    assert_eq!(
        MiniSynth::new(0.0).err(),
        Some(MiniSynthError::InvalidSampleRate)
    );
    assert_eq!(
        synth.set_parameter(44, 0.5),
        Err(MiniSynthError::InvalidParameter(44))
    );
    assert_eq!(
        synth.set_parameter(9, f32::NAN),
        Err(MiniSynthError::InvalidValue)
    );
    assert_eq!(
        synth.set_parameter(9, 1.1),
        Err(MiniSynthError::InvalidValue)
    );
    assert_eq!(synth.set_tempo_bpm(0.0), Err(MiniSynthError::InvalidValue));
    synth.set_tempo_bpm(90.0).unwrap();
    assert_eq!(
        synth.process(&mut [0.0; 2], &mut [0.0; 1]),
        Err(MiniSynthError::MismatchedBuffers)
    );
    synth.note_on(60, 1.0);
    synth.set_parameter(0, 0.0).unwrap();
    assert_eq!(render(&mut synth, 256).0, [0.0; 256]);
    // The On→Off edge cut the voice: re-enabling brings nothing back.
    synth.set_parameter(0, 1.0).unwrap();
    assert_eq!(render(&mut synth, 256).0, [0.0; 256]);
    synth.note_on(60, 1.0);
    assert!(
        render(&mut synth, 256)
            .0
            .iter()
            .any(|sample| sample.abs() > 0.01)
    );
}

/// The rising zero crossings per second of `signal`.
fn frequency(signal: &[f32], rate: f32) -> f32 {
    let rising: Vec<usize> = signal
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] < 0.0 && pair[1] >= 0.0)
        .map(|(i, _)| i)
        .collect();
    (rising.len() - 1) as f32 * rate / (rising[rising.len() - 1] - rising[0]) as f32
}

/// The played interval from A3 (220 Hz) in semitones.
fn semitones(synth: &mut MiniSynth) -> f32 {
    synth.note_on(57, 1.0);
    let (left, _) = render(synth, 22_050);
    12.0 * (frequency(&left[4_410..], 44_100.0) / 220.0).log2()
}

#[test]
fn transpose_plays_the_fused_step_at_every_menu_value() {
    // At v = f32(k/24) the step is k − 12, except
    // k = 8, 11, 14, 17, 20 → −3, 0, 1, 4, 7.
    for k in 0..=24 {
        let mut synth = MiniSynth::new(44_100.0).unwrap();
        synth
            .set_parameter(2, (f64::from(k) / 24.0) as f32)
            .unwrap();
        let expected = match k {
            8 => -3,
            11 => 0,
            14 => 1,
            17 => 4,
            20 => 7,
            _ => k - 12,
        };
        let played = semitones(&mut synth);
        assert!((played - expected as f32).abs() < 0.1, "k = {k}: {played}");
    }
}

#[test]
fn pitch_bend_is_plus_minus_two_semitones() {
    // MiniSynth bends 4w − 2 semitones.
    for (w, expected) in [(0.0, -2.0), (0.5, 0.0), (0.75, 1.0), (1.0, 2.0)] {
        let mut synth = MiniSynth::new(44_100.0).unwrap();
        // The engine's channel param 4.
        Instrument::set_controller(&mut synth, 4, w);
        let played = semitones(&mut synth);
        assert!((played - expected).abs() < 0.1, "w = {w}: {played}");
    }
}

#[test]
fn a_sixth_note_steals_one_of_five_voices() {
    // 5 voices. Six held notes at distinct pans: the oldest (hard left) is stolen.
    let mut synth = MiniSynth::new(8_000.0).unwrap();
    synth.note_on_with_pan(48, 1.0, 0.0);
    for key in [52, 55, 59, 62, 65] {
        synth.note_on_with_pan(key, 1.0, 1.0);
    }
    let (left, _) = render(&mut synth, 4_000);
    assert!(left[3_000..].iter().all(|sample| sample.abs() < 1e-3));
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
}

#[test]
fn an_open_saw_at_full_velocity_plays_the_base_wave() {
    // The base wave unchanged, rms 0.246 (about −12 dBFS) per voice.
    let mut synth = MiniSynth::new(44_100.0).unwrap();
    synth.note_on(57, 1.0);
    let (left, right) = render(&mut synth, 44_100);
    let level = rms(&left[22_050..]);
    assert!((level - 0.246).abs() < 0.006, "{level}");
    assert_eq!(left, right);
}

#[test]
fn pulse_cancels_at_modifier_zero_and_noise_scales_with_its_square() {
    // PULSE = S(φ) − S(φ + d), d = 1 − |m|: silent at m = 0 once the 0.07 offset's thump has decayed.
    let noise = |level: f32| {
        let mut synth = MiniSynth::new(44_100.0).unwrap();
        synth.set_parameter(6, 2.0 / 9.0).unwrap();
        synth.set_parameter(8, level).unwrap();
        synth.note_on(57, 1.0);
        rms(&render(&mut synth, 22_050).0[11_025..])
    };
    // What remains is the f32 DC blocker's floor: it stops tracking within half an ulp of 0.35 / 0.005.
    assert!(noise(0.0) < 1e-5, "{}", noise(0.0));
    // White noise at level v8².
    let ratio = noise(0.5) / noise(1.0);
    assert!((ratio - 0.25).abs() < 0.01, "{ratio}");
}

/// A square LFO on FRQ with LFO Amnt 0.75 (L = ±0.25, so ±3 semitones) over 2 beats, 1 s at 120 BPM.
fn square_vibrato() -> MiniSynth {
    let mut synth = MiniSynth::new(44_100.0).unwrap();
    for (index, value) in [(18, 1.0), (19, 0.5), (20, 5.5 / 11.0), (21, 0.75)] {
        synth.set_parameter(index, value).unwrap();
    }
    synth.note_on(57, 1.0);
    synth
}

#[test]
fn a_square_lfo_on_frq_swings_the_pitch_by_twelve_l_semitones() {
    // FRQ +12·L st, L = s·a²·wave; stopped, the LFO runs freely at the tempo.
    let (left, _) = render(&mut square_vibrato(), 44_100);
    let interval =
        |range: std::ops::Range<usize>| 12.0 * (frequency(&left[range], 44_100.0) / 220.0).log2();
    let (up, down) = (interval(4_410..19_845), interval(26_460..41_895));
    assert!((up - 3.0).abs() < 0.1, "{up}");
    assert!((down + 3.0).abs() < 0.1, "{down}");
}

#[test]
fn the_lfo_locks_to_the_song_position_while_playing() {
    let render_at = |start_beat: f64| {
        let mut synth = square_vibrato();
        let (mut left, mut right) = (vec![0.0; 22_050], vec![0.0; 22_050]);
        let context = ProcessContext {
            sample_rate: 44_100.0,
            tempo_bpm: 120.0,
            start_beat,
            playing: true,
        };
        Instrument::process(&mut synth, &mut left, &mut right, context);
        left
    };
    // A whole period (2 beats) later plays the same; half a period later plays the other half of the square.
    assert_eq!(render_at(0.0), render_at(2.0));
    assert_ne!(render_at(0.0), render_at(1.0));
}

fn difference(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum()
}

#[test]
fn mod_wheel_moves_filter_and_lfo_destinations() {
    let mut dry = MiniSynth::new(8_000.0).unwrap();
    let mut wheel = MiniSynth::new(8_000.0).unwrap();
    for synth in [&mut dry, &mut wheel] {
        synth.set_parameter(9, 0.35).unwrap();
        synth.set_parameter(5, 1.0).unwrap();
        synth.note_on(60, 1.0);
    }
    wheel.set_mod_wheel(1.0).unwrap();
    assert_eq!(
        wheel.set_mod_wheel(f32::NAN),
        Err(MiniSynthError::InvalidValue)
    );
    let plain = render(&mut dry, 512).0;
    let moved = render(&mut wheel, 512).0;
    assert!(difference(&plain, &moved) > 0.1);

    let mut plain = MiniSynth::new(8_000.0).unwrap();
    let mut moved = MiniSynth::new(8_000.0).unwrap();
    for synth in [&mut plain, &mut moved] {
        synth.set_parameter(4, 0.5).unwrap();
        synth.set_parameter(5, 1.0).unwrap();
        synth.set_parameter(19, 0.5).unwrap();
        synth.set_parameter(21, 0.5).unwrap();
        synth.note_on(60, 1.0);
    }
    moved.set_mod_wheel(1.0).unwrap();
    assert!(difference(&render(&mut plain, 512).0, &render(&mut moved, 512).0) > 0.1);
}

#[test]
fn osc_mod_lfo_changes_pulse_width() {
    let mut plain = MiniSynth::new(8_000.0).unwrap();
    let mut modulated = MiniSynth::new(8_000.0).unwrap();
    for synth in [&mut plain, &mut modulated] {
        synth.set_parameter(6, 2.0 / 9.0).unwrap();
        synth.set_parameter(7, 0.5).unwrap();
        synth.set_parameter(19, 1.0).unwrap();
        synth.note_on(60, 1.0);
    }
    modulated.set_parameter(21, 1.0).unwrap();
    assert!(difference(&render(&mut plain, 512).0, &render(&mut modulated, 512).0) > 0.1);
}

#[test]
fn distortion_drive_and_decimator_change_the_waveform() {
    let mut plain = MiniSynth::new(8_000.0).unwrap();
    let mut driven = MiniSynth::new(8_000.0).unwrap();
    let mut decimated = MiniSynth::new(8_000.0).unwrap();
    for synth in [&mut plain, &mut driven, &mut decimated] {
        synth.note_on(60, 1.0);
    }
    driven.set_parameter(26, 1.0).unwrap();
    driven.set_parameter(27, 1.0).unwrap();
    decimated.set_parameter(26, 1.0).unwrap();
    decimated.set_parameter(28, 1.0).unwrap();
    let base = render(&mut plain, 512).0;
    assert!(difference(&base, &render(&mut driven, 512).0) > 0.1);
    assert!(difference(&base, &render(&mut decimated, 512).0) > 0.1);
}

/// 1000 frames at 8 kHz and 480 BPM of a held note at `pan`, with DELAY on at Fdbk `feedback` when given.
fn delayed(pan: f32, feedback: Option<f32>) -> (Vec<f32>, Vec<f32>) {
    let mut synth = MiniSynth::new(8_000.0).unwrap();
    synth.set_tempo_bpm(480.0).unwrap();
    if let Some(feedback) = feedback {
        for (index, value) in [(29, 1.0), (30, 0.0), (31, feedback), (33, 1.0)] {
            synth.set_parameter(index, value).unwrap();
        }
    }
    synth.note_on_with_pan(60, 1.0, pan);
    render(&mut synth, 1000)
}

#[test]
fn delay_echoes_a_tempo_step_later_on_both_channels() {
    // 1/4 beat at 480 BPM is 250 frames; the dry signal is unchanged, the mono echo
    // (L + R in) is added to both channels, and Fdbk 0 gives no echo at all.
    let (dry_l, dry_r) = delayed(0.0, None);
    let (wet_l, wet_r) = delayed(0.0, Some(0.5));
    assert_eq!(dry_r, [0.0; 1000]);
    assert_eq!(wet_l[..250], dry_l[..250]);
    let echo_l: Vec<f32> = wet_l.iter().zip(&dry_l).map(|(w, d)| w - d).collect();
    assert!(echo_l[250..].iter().any(|x| x.abs() > 0.01));
    assert_eq!(wet_r[..250], [0.0; 250]);
    for (l, r) in echo_l.iter().zip(&wet_r) {
        assert!((l - r).abs() < 1e-6);
    }
    assert_eq!(delayed(0.0, Some(0.0)), (dry_l, dry_r));
}

#[test]
fn chorus_and_phaser_produce_distinct_effects() {
    let mut dry = MiniSynth::new(8_000.0).unwrap();
    let mut chorus = MiniSynth::new(8_000.0).unwrap();
    let mut phaser = MiniSynth::new(8_000.0).unwrap();
    for synth in [&mut dry, &mut chorus, &mut phaser] {
        synth.note_on(60, 1.0);
    }
    chorus.set_parameter(34, 1.0).unwrap();
    chorus.set_parameter(36, 1.0).unwrap();
    chorus.set_parameter(37, 1.0).unwrap();
    phaser.set_parameter(38, 1.0).unwrap();
    phaser.set_parameter(40, 1.0).unwrap();
    let base = render(&mut dry, 1024).0;
    let (chorus_l, chorus_r) = render(&mut chorus, 1024);
    let (phaser_l, _) = render(&mut phaser, 1024);
    assert!(difference(&base, &chorus_l) > 0.1);
    assert!(difference(&chorus_l, &chorus_r) > 0.1);
    assert!(difference(&base, &phaser_l) > 0.1);
}

#[test]
fn note_pan_balances_voices_before_stereo_effects() {
    let mut left = MiniSynth::new(8_000.0).unwrap();
    let mut right = MiniSynth::new(8_000.0).unwrap();
    left.note_on_with_pan(60, 1.0, 0.0);
    right.note_on_with_pan(60, 1.0, 1.0);
    let (left_l, left_r) = render(&mut left, 512);
    let (right_l, right_r) = render(&mut right, 512);
    assert!(left_l.iter().any(|sample| sample.abs() > 0.01));
    assert_eq!(left_r, [0.0; 512]);
    assert_eq!(right_l, [0.0; 512]);
    assert_eq!(left_l, right_r);
}

#[test]
fn overlapping_same_pitch_notes_release_together() {
    // POLY releases every voice on that key.
    let mut synth = MiniSynth::new(8_000.0).unwrap();
    synth.set_parameter(25, 0.0).unwrap();
    synth.note_on_with_pan(60, 1.0, 0.0);
    synth.note_on_with_pan(60, 1.0, 1.0);
    render(&mut synth, 128);
    synth.note_off(60);
    let (left, right) = render(&mut synth, 4096);
    assert!(left[3000..].iter().all(|sample| sample.abs() < 0.001));
    assert!(right[3000..].iter().all(|sample| sample.abs() < 0.001));
}

#[test]
fn all_notes_off_releases_every_overlapping_voice() {
    let mut synth = MiniSynth::new(8_000.0).unwrap();
    synth.set_parameter(25, 0.0).unwrap();
    synth.note_on_with_pan(60, 1.0, 0.0);
    synth.note_on_with_pan(60, 1.0, 1.0);
    synth.note_on(64, 1.0);
    render(&mut synth, 128);
    synth.all_notes_off();
    // The gains fade at 0.005 per sample once the amp envelope is below 1e-5.
    let (left, right) = render(&mut synth, 4096);
    assert!(left[3000..].iter().all(|sample| sample.abs() < 0.001));
    assert!(right[3000..].iter().all(|sample| sample.abs() < 0.001));
}

#[test]
fn reset_clears_voices_and_effect_tails_but_keeps_controls() {
    let mut synth = MiniSynth::new(8_000.0).unwrap();
    synth.set_parameter(6, 2.0 / 9.0).unwrap();
    synth.set_parameter(7, 0.5).unwrap();
    synth.set_parameter(4, 1.0).unwrap();
    synth.set_parameter(5, 1.0).unwrap();
    synth.set_parameter(29, 1.0).unwrap();
    synth.set_parameter(31, 0.7).unwrap();
    synth.set_parameter(33, 1.0).unwrap();
    synth.set_parameter(34, 1.0).unwrap();
    synth.set_parameter(38, 1.0).unwrap();
    synth.set_mod_wheel(0.8).unwrap();
    synth.set_tempo_bpm(480.0).unwrap();
    synth.note_on(60, 1.0);
    render(&mut synth, 2_000);
    synth.reset();
    assert_eq!(synth.parameter(29), Some(1.0));
    assert_eq!(
        render(&mut synth, 2_000),
        (vec![0.0; 2_000], vec![0.0; 2_000])
    );

    let mut reference = MiniSynth::new(8_000.0).unwrap();
    for index in 0..44 {
        reference
            .set_parameter(index, synth.parameter(index).unwrap())
            .unwrap();
    }
    reference.set_mod_wheel(0.8).unwrap();
    reference.set_tempo_bpm(480.0).unwrap();
    // The same 2000 silent frames: the song position (LFO and phaser phase) and the control-block grid run on.
    render(&mut reference, 2_000);
    synth.note_on(60, 1.0);
    reference.note_on(60, 1.0);
    assert_eq!(render(&mut synth, 1_200), render(&mut reference, 1_200));
}

/// FNV-1a over the bits of both channels: a fingerprint of the exact output.
fn fingerprint(left: &[f32], right: &[f32]) -> u64 {
    let bytes = left
        .iter()
        .chain(right)
        .flat_map(|sample| sample.to_bits().to_le_bytes());
    bytes.fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// 2 s at 48 kHz in odd blocks: poly with stealing and same-key notes, mono, lead with glide, every OSC Kind,
/// noise, the filter (wide open too) with its envelope and tracking, all LFO shapes and destinations, the mod
/// wheel, Enable off and on, the built-in FX and a reset. The hash pins every sample (debug and release give the
/// same hash).
#[test]
fn golden_render_is_unchanged() {
    const FRAMES: usize = 96_000;
    let mut synth = MiniSynth::new(48_000.0).unwrap();
    for (index, value) in [
        (3, 0.02),
        (7, 0.3),
        (8, 0.1),
        (9, 0.55),
        (10, 0.7),
        (11, 0.6),
        (12, 0.5),
        (13, 0.01),
        (14, 0.2),
        (15, 0.3),
        (16, 0.1),
        (17, 0.85),
        (21, 0.8),
        (22, 0.005),
        (23, 0.3),
        (24, 0.6),
        (25, 0.05),
    ] {
        synth.set_parameter(index, value).unwrap();
    }
    synth.set_tempo_bpm(133.0).unwrap();
    let (mut left, mut right) = (vec![0.0; FRAMES], vec![0.0; FRAMES]);
    let note = |block: usize| {
        if block % 6 == 3 {
            60
        } else {
            36 + (block * 7 % 41) as u8
        }
    };
    let mut start = 0;
    let mut block = 0;
    while start < FRAMES {
        let set = |synth: &mut MiniSynth, index, value| synth.set_parameter(index, value).unwrap();
        set(&mut synth, 6, (block % 10) as f32 / 9.0);
        let velocity = 0.4 + 0.06 * (block * 5 % 11) as f32;
        let pan = (block % 5) as f32 / 4.0;
        match block {
            // Poly: one note per block, each held for 18 blocks, so from block 5 on every note steals.
            0..=24 => {
                synth.note_on_with_pan(note(block), velocity, pan);
                if block >= 18 {
                    synth.note_off(note(block - 18));
                }
                if block == 20 {
                    set(&mut synth, 4, 0.5);
                    set(&mut synth, 5, 0.6);
                    synth.set_mod_wheel(0.7).unwrap();
                }
            }
            25 => synth.all_notes_off(),
            // Mono, LFO to frequency; the filter opens fully for a while.
            28..=49 => {
                if block == 28 {
                    set(&mut synth, 1, 0.5);
                    set(&mut synth, 19, 0.5);
                }
                if block == 40 {
                    set(&mut synth, 9, 1.0);
                    set(&mut synth, 17, 0.5);
                }
                if block == 45 {
                    set(&mut synth, 9, 0.4);
                    set(&mut synth, 17, 0.2);
                }
                if block % 3 == 1 {
                    synth.note_on_with_pan(note(block), velocity, pan);
                } else if block % 3 == 0 {
                    synth.note_off(note(block - 2));
                }
            }
            // Lead with glide, LFO to OSC Mod (saw, then square); overlapping notes slide.
            50..=79 => {
                if block == 50 {
                    set(&mut synth, 1, 1.0);
                    set(&mut synth, 3, 0.05);
                    set(&mut synth, 18, 0.5);
                    set(&mut synth, 19, 1.0);
                    set(&mut synth, 4, 1.0);
                }
                if block == 60 {
                    set(&mut synth, 18, 1.0);
                    set(&mut synth, 26, 1.0);
                    set(&mut synth, 27, 0.2);
                    set(&mut synth, 28, 0.1);
                }
                if block == 70 {
                    for (index, value) in [(29, 1.0), (30, 0.3), (31, 0.4), (34, 1.0), (38, 1.0)] {
                        set(&mut synth, index, value);
                    }
                }
                if block % 4 == 2 {
                    synth.note_on_with_pan(note(block), velocity, pan);
                }
                if block % 4 == 0 {
                    synth.note_off(note(block - 6));
                }
            }
            // Poly chords, Enable off for one block, then a reset and one last note.
            80..=89 => {
                if block == 80 {
                    set(&mut synth, 1, 0.0);
                    set(&mut synth, 18, 0.0);
                    set(&mut synth, 19, 0.0);
                    set(&mut synth, 4, 0.0);
                }
                set(&mut synth, 0, if block == 85 { 0.0 } else { 1.0 });
                for offset in [0, 4, 7] {
                    synth.note_on_with_pan(note(block) + offset, velocity, pan);
                    if block >= 82 {
                        synth.note_off(note(block - 2) + offset);
                    }
                }
            }
            90 => synth.reset(),
            91 => synth.note_on(64, 0.9),
            _ => {}
        }
        let end = (start + 997 + block % 7 * 13).min(FRAMES);
        synth
            .process(&mut left[start..end], &mut right[start..end])
            .unwrap();
        start = end;
        block += 1;
    }
    assert!(left.iter().chain(&right).all(|sample| sample.is_finite()));
    assert!(left.iter().any(|sample| sample.abs() > 0.01));
    assert_eq!(fingerprint(&left, &right), 0x6916_0e0c_6140_f206);
}

#[test]
fn lfo_rate_curve_passes_through_the_captured_points() {
    // The DW Sampler's LFO Rate curve (Hz, v = 0, .05 … 1), shared by SuperSaw.
    let hz = [
        0.03, 0.04, 0.05, 0.05, 0.06, 0.08, 0.10, 0.12, 0.15, 0.20, 0.26, 0.36, 0.51, 0.76, 1.19,
        1.99, 3.66, 7.43, 15.79, 26.97, 30.00,
    ];
    // The round-8 law 1 / (30·(1 − v)³ + 0.033333) Hz hits every point to 2 decimals.
    for (i, hz) in hz.into_iter().enumerate() {
        let v = i as f32 / 20.0;
        assert_eq!(format!("{:.2}", lfo_rate_hz(v)), format!("{hz:.2}"), "{v}");
    }
    assert_eq!(format!("{:.2}", lfo_rate_hz(0.925)), "21.74");
}

#[test]
fn lfo_attack_law_matches_the_captured_times_at_44_1_khz() {
    // Section 4.2 #33: the fade-in rises by c per sample, so it takes t = 1 / (c·Fs) seconds.
    let seconds = |v: f32| 1.0 / (lfo_attack_increment(v) * 44_100.0);
    assert_eq!(lfo_attack_increment(0.0), 1.0);
    assert_eq!(lfo_attack_increment(0.0009), 1.0);
    assert_eq!((1000.0 * seconds(0.25)) as i32, 387);
    assert_eq!(format!("{:.2}", seconds(0.5)), "4.31");
    assert_eq!(format!("{:.2}", seconds(1.0)), "56.69");
    // The law is per sample, so 48 kHz is shorter by 44.1/48.
    let at_48k = 1.0 / (lfo_attack_increment(1.0) * 48_000.0);
    assert!((at_48k - seconds(1.0) * 44_100.0 / 48_000.0).abs() < 1e-3);
}

#[test]
fn tempo_steps_read_each_menu_as_beats() {
    // "n/d" is n/d beats: GMSynth's UI shows its "1/1" as Beat and "4/1" as Bar.
    let step = |list: &[f32], k: usize| tempo_step(list, k as f32 / (list.len() - 1) as f32);
    assert_eq!(LFO_SYNC_BEATS.len(), 10);
    assert_eq!(
        [
            step(&LFO_SYNC_BEATS, 0),
            step(&LFO_SYNC_BEATS, 4),
            step(&LFO_SYNC_BEATS, 9)
        ],
        [0.125, 1.0, 32.0]
    );
    // SuperSaw's LFO Beat menu lists 3/4 before 1/2.
    assert_eq!(
        [step(&SUPERSAW_LFO_BEATS, 2), step(&SUPERSAW_LFO_BEATS, 3)],
        [0.75, 0.5]
    );
    assert_eq!(GMSYNTH_RATE_BEATS[4], 1.0);
    assert_eq!(GMSYNTH_RATE_BEATS[6], 4.0);
    assert_eq!(step(&GMSYNTH_RATE_BEATS, 9), 32.0);
    // The defaults: SuperSaw ARP Rate 0.2 is 1/4, Delay Time 0.775 is 3/4.
    assert_eq!(tempo_step(&ARP_RATE_BEATS, 0.2), 0.25);
    assert_eq!(tempo_step(&SUPERSAW_DELAY_BEATS, 0.775), 0.75);
    // Values between steps truncate, as the round-8 DSP's int(9·v).
    assert_eq!(tempo_step(&LFO_SYNC_BEATS, 0.5), 1.0);
    assert_eq!(tempo_step(&LFO_SYNC_BEATS, 0.43), 0.75);
}
