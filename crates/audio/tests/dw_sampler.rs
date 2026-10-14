//! DW Sampler playback, rendered offline. Constant (DC) zones make
//! the gain laws exact: the output is value × level² × velocity × zone gain × pan.

use std::sync::Arc;

use honeydew_audio::ProcessContext;
use honeydew_audio::dw_sampler::{DwSampler, DwZone};
use honeydew_audio::dwp::{Midi, Zone};
use honeydew_audio::instrument::Instrument;
use honeydew_audio::sample::{SampleData, SampleMeta};
use honeydew_audio::voice::lfo_rate_hz;

const RATE: f32 = 48_000.0;

fn context(start_beat: f64, playing: bool) -> ProcessContext {
    ProcessContext {
        sample_rate: RATE,
        tempo_bpm: 120.0,
        start_beat,
        playing,
    }
}

fn sample(rate: u32, left: Vec<f32>) -> Arc<SampleData> {
    Arc::new(SampleData {
        sample_rate: rate,
        left: left.into(),
        right: None,
        meta: SampleMeta::default(),
    })
}

fn dc(value: f32, frames: usize) -> Arc<SampleData> {
    sample(48_000, vec![value; frames])
}

fn sine(hz: f64, rate: u32, seconds: f64) -> Arc<SampleData> {
    let frames = (f64::from(rate) * seconds) as usize;
    let wave =
        (0..frames).map(|n| (std::f64::consts::TAU * hz * n as f64 / f64::from(rate)).sin() as f32);
    sample(rate, wave.collect())
}

/// Frame index / 1000: the value read shows the read position.
fn ramp(frames: usize) -> Arc<SampleData> {
    sample(48_000, (0..frames).map(|n| n as f32 / 1000.0).collect())
}

fn zone(sample: Arc<SampleData>, zone: Zone) -> DwZone {
    DwZone { zone, sample }
}

fn keys(low: u8, high: u8) -> Zone {
    Zone {
        key_low: low,
        key_high: high,
        ..Zone::default()
    }
}

fn sampler(zones: Vec<DwZone>) -> DwSampler {
    DwSampler::new(RATE, zones, Midi::default())
}

fn render(dw: &mut DwSampler, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    for (l, r) in left.chunks_mut(512).zip(right.chunks_mut(512)) {
        dw.process(l, r, context(0.0, false));
    }
    (left, right)
}

/// The first `frames` of the left channel after `key` starts at `velocity`.
fn play(dw: &mut DwSampler, key: u8, velocity: f32, frames: usize) -> Vec<f32> {
    dw.note_on(key, velocity, 0.5);
    render(dw, frames).0
}

/// The frequency from the first to the last rising zero crossing, interpolated.
fn pitch(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&n| x[n - 1] <= 0.0 && x[n] > 0.0)
        .map(|n| (n - 1) as f64 + f64::from(-x[n - 1] / (x[n] - x[n - 1])))
        .collect();
    assert!(crossings.len() > 2, "no pitch in {} frames", x.len());
    (crossings.len() - 1) as f64 * f64::from(RATE) / (crossings[crossings.len() - 1] - crossings[0])
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance * expected.abs(),
        "{actual} vs {expected}"
    );
}

#[test]
fn zones_are_chosen_by_key_and_velocity() {
    let layer = |low: u8, high: u8| Zone {
        root: 72,
        vel_low: low,
        vel_high: high,
        ..keys(60, 128)
    };
    let zones = vec![
        zone(dc(0.1, 9600), keys(0, 59)),
        zone(dc(0.2, 9600), layer(0, 63)),
        zone(dc(0.4, 9600), layer(64, 127)),
    ];
    let level = |key: u8, velocity: f32| play(&mut sampler(zones.clone()), key, velocity, 8)[4];
    assert_eq!(level(59, 1.0), 0.1);
    // round(127·vel) picks the layer; amplitude is linear in velocity.
    assert_eq!(level(60, 0.49), 0.2 * 0.49); // 62.2 → 62
    assert_eq!(level(60, 0.5), 0.4 * 0.5); // 63.5 → 64
    assert_eq!(level(127, 0.6), 0.4 * 0.6);
    assert_eq!(level(60, 1.0), 0.4);
    // No zone, no sound.
    let mut narrow = sampler(vec![zone(dc(0.5, 9600), keys(10, 20))]);
    assert_eq!(play(&mut narrow, 21, 1.0, 8), [0.0; 8]);
}

#[test]
fn pitch_follows_the_step_law() {
    // A 441 Hz sine recorded at 44.1 kHz, root 48, played at 48 kHz.
    let piano = vec![
        zone(sine(441.0, 44_100, 2.0), keys(0, 59)),
        zone(
            sine(300.0, 44_100, 2.0),
            Zone {
                root: 60,
                ..keys(60, 128)
            },
        ),
    ];
    let measure = |set: &[(usize, f32)], bend: Option<f32>, midi: Midi, key: u8| {
        let mut dw = DwSampler::new(RATE, piano.clone(), midi);
        for &(index, value) in set {
            dw.set_parameter(index, value);
        }
        if let Some(v) = bend {
            dw.set_controller(4, v);
        }
        pitch(&play(&mut dw, key, 1.0, 24_000)[2400..])
    };
    let plain = Midi::default();
    close(measure(&[], None, plain, 48), 441.0, 1e-4);
    close(
        measure(&[], None, plain, 55),
        441.0 * 2f64.powf(7.0 / 12.0),
        1e-4,
    );
    // Pitch +12 (trunc(48·0.75 − 24)): key 48 now matches the key-60 zone, and the step adds the 12.
    close(measure(&[(3, 0.75)], None, plain, 48), 300.0, 1e-4);
    close(
        measure(&[(3, 0.75)], None, plain, 47),
        441.0 * 2.0 * 2f64.powf(-1.0 / 12.0),
        1e-4,
    );
    // Fine at 1 is +1 semitone.
    close(
        measure(&[(4, 1.0)], None, plain, 48),
        441.0 * 2f64.powf(1.0 / 12.0),
        1e-4,
    );
    // Bend (2v − 1) × PB range: 2 semitones on a new module, the CSTM's range otherwise.
    close(
        measure(&[], Some(1.0), plain, 48),
        441.0 * 2f64.powf(2.0 / 12.0),
        1e-4,
    );
    let seven = Midi {
        pb_range: 7,
        attack_flag: 0,
    };
    close(
        measure(&[], Some(0.0), seven, 48),
        441.0 * 2f64.powf(-7.0 / 12.0),
        1e-4,
    );
}

#[test]
fn a_random_pick_never_repeats_the_previous_pick_for_that_key() {
    let zones = |levels: &[f32]| {
        levels
            .iter()
            .map(|&v| zone(dc(v, 9600), Zone::default()))
            .collect::<Vec<_>>()
    };
    let mut dw = sampler(zones(&[0.1, 0.2]));
    let pick = |dw: &mut DwSampler, key| {
        let level = play(dw, key, 1.0, 4)[2];
        dw.reset();
        level
    };
    let first = pick(&mut dw, 60);
    for round in 0..20 {
        // Another key's picks do not disturb key 60's.
        pick(&mut dw, 61);
        pick(&mut dw, 61);
        let expected = if round % 2 == 0 { 0.3 - first } else { first };
        assert!((pick(&mut dw, 60) - expected).abs() < 1e-6, "round {round}");
    }
    let mut three = sampler(zones(&[0.1, 0.2, 0.3]));
    let picks: Vec<f32> = (0..60).map(|_| pick(&mut three, 64)).collect();
    assert!(picks.windows(2).all(|w| w[0] != w[1]), "{picks:?}");
    for level in [0.1, 0.2, 0.3] {
        assert!(picks.contains(&level), "{level} never picked: {picks:?}");
    }
}

#[test]
fn loops_run_forward_over_start_to_end_while_loop_is_on() {
    let looped = |mode: i32, (start, end): (u32, u32), meta: Option<(u32, u32)>| {
        let mut sample = (*ramp(1000)).clone();
        sample.meta.loop_frames = meta;
        let zone = Zone {
            loop_mode: mode,
            loop_start: start,
            loop_end: end,
            ..Zone::default()
        };
        sampler(vec![DwZone {
            zone,
            sample: Arc::new(sample),
        }])
    };
    // Root key, same rate: one frame per frame, so the output is the read position / 1000.
    let out = play(&mut looped(2, (200, 400), None), 48, 1.0, 3000);
    assert_eq!(out[399], 0.399);
    assert_eq!(out[400], 0.2);
    assert_eq!(out[2999], (200 + (2999 - 200) % 200) as f32 / 1000.0);
    // Loop (p9) off ignores the zone loop; so does a mode ≤ 1. The voice ends at the sample's end.
    let mut off = looped(2, (200, 400), None);
    off.set_parameter(9, 0.0);
    let out = play(&mut off, 48, 1.0, 1200);
    assert_eq!((out[999], out[1000], out[1199]), (0.999, 0.0, 0.0));
    let out = play(&mut looped(1, (200, 400), None), 48, 1.0, 1200);
    assert_eq!(out[1100], 0.0);
    // Start = end takes the file's smpl loop; none, no loop.
    let out = play(&mut looped(2, (0, 0), Some((100, 300))), 48, 1.0, 1200);
    assert_eq!((out[299], out[300], out[1100]), (0.299, 0.1, 0.1));
    let out = play(&mut looped(2, (5, 5), None), 48, 1.0, 1200);
    assert_eq!(out[1100], 0.0);
    // A user sample: root 48 over all keys, looping over its smpl loop.
    let mut sample = (*ramp(1000)).clone();
    sample.meta.loop_frames = Some((100, 300));
    let mut user = sampler(vec![zone(Arc::new(sample), Zone::user_sample())]);
    assert_eq!(play(&mut user, 48, 1.0, 1200)[1100], 0.1);
}

#[test]
fn reverse_plays_from_the_end_and_never_loops() {
    let looping = Zone {
        loop_mode: 2,
        loop_start: 200,
        loop_end: 400,
        ..Zone::default()
    };
    let mut dw = sampler(vec![zone(ramp(1000), looping)]);
    dw.set_parameter(8, 1.0);
    let out = play(&mut dw, 48, 1.0, 1200);
    assert_eq!((out[0], out[500], out[999]), (0.999, 0.499, 0.0));
    assert_eq!(out[1000..], [0.0; 200]);
}

#[test]
fn zone_gain_applies_only_while_loop_is_on() {
    let half = Zone {
        gain: 0.5,
        ..Zone::default()
    };
    let mut on = sampler(vec![zone(dc(0.8, 9600), half)]);
    assert_eq!(play(&mut on, 60, 1.0, 4)[2], 0.4);
    let mut off = sampler(vec![zone(dc(0.8, 9600), half)]);
    off.set_parameter(9, 0.0);
    assert_eq!(play(&mut off, 60, 1.0, 4)[2], 0.8);
}

/// The amp level at `frame` of a held DC-1 note: the output is level².
fn amp(set: &[(usize, f32)], zone_floats: [f32; 4], midi: Midi, frames: usize) -> Vec<f32> {
    let held = Zone {
        floats: zone_floats,
        ..Zone::default()
    };
    let mut dw = DwSampler::new(RATE, vec![zone(dc(1.0, 480_000), held)], midi);
    for &(index, value) in set {
        dw.set_parameter(index, value);
    }
    play(&mut dw, 60, 1.0, frames)
        .into_iter()
        .map(f32::sqrt)
        .collect()
}

#[test]
fn the_amp_envelope_is_linear_with_full_scale_time_0_001_plus_10_v4() {
    let floats = Zone::default().floats;
    let t = |v: f32| 0.001 + 10.0 * v.powi(4);
    let frames = |seconds: f32| (seconds * RATE) as usize;
    // Attack 0.5: 0.626 s from 0 to 1.
    let attack = amp(&[(1, 0.5)], floats, Midi::default(), frames(1.0));
    close(f64::from(attack[frames(t(0.5) / 2.0)]), 0.5, 1e-3);
    assert_eq!(attack[frames(t(0.5)) + 100], 1.0);
    // Decay 0.5 to Sustain 0.5: from 1 down at full scale per T, so 0.75 a quarter T in.
    let decay = amp(&[(5, 0.5), (6, 0.5)], floats, Midi::default(), frames(1.0));
    assert_eq!(decay[0], 1.0, "Attack 0 is instant with the MIDI flag 0");
    close(f64::from(decay[frames(t(0.5) / 4.0)]), 0.75, 1e-3);
    close(f64::from(decay[frames(0.9)]), 0.5, 1e-6);
    // The zone's 2nd float ≤ 0: no Decay ramp, the level jumps to Sustain.
    let jump = amp(
        &[(5, 0.5), (6, 0.5)],
        [0.0, 0.0, 1.0, 0.1],
        Midi::default(),
        16,
    );
    close(f64::from(jump[1]), 0.5, 1e-6);
    // Attack at v = 0 takes 1 ms once the MIDI flag is set (any CSTM sets it).
    let floored = amp(
        &[],
        floats,
        Midi {
            pb_range: 4,
            attack_flag: 1,
        },
        100,
    );
    close(f64::from(floored[23]), 0.5, 0.05);
    assert_eq!(floored[60], 1.0);
}

#[test]
fn release_takes_its_level_times_t() {
    let release = |sustain: f32| {
        let mut dw = sampler(vec![zone(dc(1.0, 480_000), Zone::default())]);
        for (index, value) in [(2, 0.5), (5, 0.0), (6, sustain)] {
            dw.set_parameter(index, value);
        }
        play(&mut dw, 60, 1.0, 4800);
        dw.note_off(60);
        render(&mut dw, 48_000)
            .0
            .into_iter()
            .map(f32::sqrt)
            .collect::<Vec<_>>()
    };
    let t = 0.001 + 10.0 * 0.5f32.powi(4);
    let full = release(1.0);
    close(f64::from(full[(t / 2.0 * RATE) as usize]), 0.5, 1e-3);
    let half = release(0.5);
    close(f64::from(half[(t / 4.0 * RATE) as usize]), 0.25, 2e-3);
    assert_eq!(half[(t / 2.0 * RATE) as usize + 100], 0.0);
}

/// A looping DC zone (so high keys' fast reads never run out).
fn looping_dc(value: f32, zone: Zone) -> DwZone {
    let looping = Zone {
        loop_mode: 2,
        loop_start: 0,
        loop_end: 9600,
        ..zone
    };
    DwZone {
        zone: looping,
        sample: dc(value, 9600),
    }
}

#[test]
fn voices_stop_at_128_with_no_stealing() {
    // Each key has its own level, so a stolen voice would change the sum.
    let level = |key: u8| f32::from(key + 1) / 16_384.0;
    let zones = (0..128)
        .map(|key| looping_dc(level(key), keys(key, key)))
        .collect();
    let mut dw = sampler(zones);
    for key in 0..128 {
        dw.note_on(key, 1.0, 0.5);
    }
    let full = render(&mut dw, 4).0[2];
    close(f64::from(full), 128.0 * 129.0 / 2.0 / 16_384.0, 1e-5);
    dw.note_on(60, 1.0, 0.5);
    assert_eq!(render(&mut dw, 4).0[2], full, "the 129th note is dropped");
    dw.note_off(0);
    render(&mut dw, 4800);
    dw.note_on(60, 1.0, 0.5);
    let again = render(&mut dw, 4).0[2];
    close(
        f64::from(again),
        f64::from(full - level(0) + level(60)),
        1e-5,
    );
}

fn piano() -> DwSampler {
    sampler(vec![zone(sine(441.0, 44_100, 4.0), Zone::default())])
}

#[test]
fn mono_is_one_voice_with_last_note_priority() {
    let mut dw = piano();
    dw.set_parameter(11, 1.0);
    let first = play(&mut dw, 48, 1.0, 9600);
    close(pitch(&first[480..]), 441.0, 1e-4);
    let both = play(&mut dw, 55, 1.0, 9600);
    close(pitch(&both[480..]), 441.0 * 2f64.powf(7.0 / 12.0), 1e-4);
    assert!(both.iter().all(|s| s.abs() <= 1.0), "two voices sound");
    dw.note_off(55);
    let back = render(&mut dw, 9600).0;
    close(pitch(&back[480..]), 441.0, 1e-4);
    dw.note_off(48);
    let released = render(&mut dw, 9600).0;
    assert_eq!(released[4800..], [0.0; 4800]);
}

#[test]
fn slide_glides_from_the_last_note() {
    // Slide 1: t = (1/2)³ = 0.125 s.
    let glide = |legato: f32| {
        let mut dw = piano();
        dw.set_parameter(7, 1.0);
        dw.set_parameter(10, legato);
        play(&mut dw, 36, 1.0, 480);
        dw.note_off(36);
        render(&mut dw, 4800);
        play(&mut dw, 48, 1.0, 48_000)
    };
    // Legato off: every note glides from the last (an octave below), here with no key held.
    let off = glide(0.0);
    assert!(pitch(&off[..1200]) < 300.0, "{}", pitch(&off[..1200]));
    close(pitch(&off[43_200..]), 441.0, 1e-3);
    // Legato on: only while another key is held.
    let on = glide(1.0);
    close(pitch(&on[..1200]), 441.0, 1e-3);
}

/// The square LFO on Level at amount 1 swings the gain between 0 and 2: the period between rising crossings of 1.
fn level_lfo_period(set: &[(usize, f32)], playing: bool) -> f64 {
    let held = Zone {
        loop_mode: 2,
        loop_start: 0,
        loop_end: 48_000,
        ..Zone::default()
    };
    let mut dw = sampler(vec![zone(dc(1.0, 48_000), held)]);
    for &(index, value) in [(27, 2.0 / 3.0), (31, 1.0), (32, 2.0 / 3.0)]
        .iter()
        .chain(set)
    {
        dw.set_parameter(index, value);
    }
    dw.note_on(60, 1.0, 0.5);
    let frames = 5 * RATE as usize;
    let mut out = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let beats_per_frame = 120.0 / 60.0 / f64::from(RATE);
    for (i, (l, r)) in out.chunks_mut(333).zip(right.chunks_mut(333)).enumerate() {
        // The song position holds still while the transport is stopped, as the engine sends it.
        let beat = if playing {
            i as f64 * 333.0 * beats_per_frame
        } else {
            1.3
        };
        dw.process(l, r, context(beat, playing));
    }
    let rises: Vec<usize> = (1..frames)
        .filter(|&n| out[n - 1] < 1.0 && out[n] >= 1.0)
        .collect();
    assert!(rises.len() > 3, "{rises:?}");
    (rises[rises.len() - 1] - rises[0]) as f64 / (rises.len() - 1) as f64 / f64::from(RATE)
}

#[test]
fn lfo_hertz_and_tmp_sync_rates() {
    close(
        level_lfo_period(&[(28, 0.75)], false),
        1.0 / f64::from(lfo_rate_hz(0.75)),
        0.01,
    );
    close(
        level_lfo_period(&[(28, 0.9)], false),
        1.0 / f64::from(lfo_rate_hz(0.9)),
        0.01,
    );
    // Tmp Sync 1/4 (int(9·v) = 1) at 120 BPM: a quarter beat, 0.125 s, locked to the song while it plays.
    let sync = [(29, 1.0), (30, 1.5 / 9.0)];
    close(level_lfo_period(&sync, true), 0.125, 0.01);
    close(level_lfo_period(&sync, false), 0.125, 0.01);
}

#[test]
fn the_filter_types_shape_a_5_khz_tone() {
    let rms = |filter: f32, cutoff: f32| {
        let mut dw = sampler(vec![zone(sine(5000.0, 48_000, 1.0), Zone::default())]);
        dw.set_parameter(16, filter);
        dw.set_parameter(12, cutoff);
        let out = play(&mut dw, 48, 1.0, 24_000);
        (out[4800..].iter().map(|s| s * s).sum::<f32>() / 19_200.0).sqrt()
    };
    let dry = rms(0.0, 0.0);
    close(f64::from(dry), 0.5f64.sqrt(), 1e-3);
    // Cutoff v → 2^(4 + 10.4v) Hz: 0.4775 is about 500 Hz, 1 is 21.6 kHz.
    assert!(
        rms(1.0 / 3.0, 0.4775) < 0.001 * dry,
        "a 4-pole lowpass at 500 Hz"
    );
    assert!(rms(1.0 / 3.0, 1.0) > 0.9 * dry, "lowpass at 21.6 kHz");
    assert!(rms(1.0, 0.4775) > 0.9 * dry, "highpass at 500 Hz");
    assert!(rms(1.0, 1.0) < 0.2 * dry);
    // Bandpass: constant skirt gain, Q 0.5 at Res 0, so a tone at the cutoff passes at 0.5.
    let at_5k = (5000f32.log2() - 4.0) / 10.4;
    close(f64::from(rms(2.0 / 3.0, at_5k)), f64::from(0.5 * dry), 0.02);
    assert!(rms(2.0 / 3.0, 0.4775) < 0.15 * dry);
}

#[test]
fn the_frequency_envelope_reaches_four_octaves() {
    let measure = |amount: f32| {
        let mut dw = sampler(vec![zone(sine(200.0, 48_000, 2.0), Zone::default())]);
        dw.set_parameter(26, amount);
        pitch(&play(&mut dw, 48, 1.0, 9600)[480..])
    };
    // Attack 0, Sustain 1: e sits at 1, ratio 2^(4·(2·Amt − 1)³).
    close(measure(1.0), 3200.0, 1e-3);
    close(measure(0.5), 200.0, 1e-4);
    close(measure(0.75), 200.0 * 2f64.powf(0.5), 1e-3);
}

#[test]
fn note_pan_follows_the_shared_law() {
    let pan = |pan: f32| {
        let mut dw = sampler(vec![zone(dc(0.5, 9600), Zone::default())]);
        dw.note_on(60, 1.0, pan);
        let (l, r) = render(&mut dw, 4);
        (l[2], r[2])
    };
    assert_eq!(pan(0.0), (0.5, 0.0));
    assert_eq!(pan(1.0), (0.0, 0.5));
    assert_eq!(pan(0.75), (0.25, 0.5));
}

#[test]
fn enable_off_cuts_every_voice() {
    let mut dw = sampler(vec![zone(dc(0.5, 96_000), Zone::default())]);
    dw.note_on(60, 1.0, 0.5);
    render(&mut dw, 64);
    dw.set_parameter(0, 0.0);
    dw.set_parameter(0, 1.0);
    assert_eq!(render(&mut dw, 64).0, [0.0; 64]);
}

#[test]
fn block_size_does_not_change_the_output() {
    let build = || {
        let looping = Zone {
            loop_mode: 2,
            loop_start: 1000,
            loop_end: 30_000,
            ..Zone::default()
        };
        let mut dw = sampler(vec![zone(sine(300.0, 44_100, 1.0), looping)]);
        for (index, value) in [
            (7, 0.6),
            (16, 1.0 / 3.0),
            (12, 0.6),
            (13, 0.7),
            (17, 0.3),
            (21, 0.8),
            (26, 0.6),
            (28, 0.8),
            (31, 0.9),
            (33, 0.2),
        ] {
            dw.set_parameter(index, value);
        }
        dw.note_on(48, 0.8, 0.3);
        dw.note_on(55, 0.6, 0.7);
        dw
    };
    let mut whole = (vec![0.0; 6000], vec![0.0; 6000]);
    build().process(&mut whole.0, &mut whole.1, context(0.0, false));
    let mut parts = (vec![0.0; 6000], vec![0.0; 6000]);
    let mut dw = build();
    let mut start = 0;
    for size in [1, 7, 333, 1023, 4636] {
        let end = start + size;
        dw.process(
            &mut parts.0[start..end],
            &mut parts.1[start..end],
            context(0.0, false),
        );
        start = end;
    }
    assert!(whole.0.iter().any(|s| s.abs() > 0.01));
    assert_eq!(whole, parts);
}

#[test]
fn pitch_steps_play_and_match_on_the_fused_law() {
    // s = trunc(fma_f32(v, 48, −24)), so v = f32(25/48) is +0 and v = f32(16/48) is −7, in the step and
    // in zone matching. The key-49 zone has its own tone.
    let zones = vec![
        zone(sine(441.0, 44_100, 2.0), keys(0, 48)),
        zone(
            sine(300.0, 44_100, 2.0),
            Zone {
                root: 49,
                ..keys(49, 128)
            },
        ),
    ];
    let measure = |k: f32, key: u8| {
        let mut dw = sampler(zones.clone());
        dw.set_parameter(3, k / 48.0);
        pitch(&play(&mut dw, key, 1.0, 9600)[480..])
    };
    // +0: key 48 stays in the key-48 zone at its root.
    close(measure(25.0, 48), 441.0, 1e-4);
    // +2 at k = 26: key 47 + 2 answers the key-49 zone, played at its root.
    close(measure(26.0, 47), 300.0, 1e-4);
    // −7 (not −8) at k = 16: key 56 − 7 answers the key-49 zone, played at its root.
    close(measure(16.0, 56), 300.0, 1e-4);
}

#[test]
fn reverse_loop_legato_and_mono_are_on_at_any_nonzero_value() {
    // p8–p11 are on at v ≠ 0; Enable is on at v ≥ 0.5.
    for v in [0.001, 0.5] {
        let mut reverse = sampler(vec![zone(ramp(1000), Zone::default())]);
        reverse.set_parameter(8, v);
        assert_eq!(play(&mut reverse, 48, 1.0, 4)[0], 0.999, "Reverse at {v}");
        let half = Zone {
            gain: 0.5,
            ..Zone::default()
        };
        let mut looping = sampler(vec![zone(dc(0.8, 960), half)]);
        looping.set_parameter(9, v);
        assert_eq!(play(&mut looping, 60, 1.0, 4)[2], 0.4, "Loop at {v}");
        let mut mono = piano();
        mono.set_parameter(11, v);
        play(&mut mono, 48, 1.0, 960);
        let both = play(&mut mono, 55, 1.0, 9600);
        assert!(both.iter().all(|s| s.abs() <= 1.0), "Mono at {v}");
        // Legato on: a note with no other key held does not glide.
        let mut legato = piano();
        legato.set_parameter(7, 1.0);
        legato.set_parameter(10, v);
        play(&mut legato, 36, 1.0, 480);
        legato.note_off(36);
        render(&mut legato, 4800);
        close(pitch(&play(&mut legato, 48, 1.0, 1200)), 441.0, 1e-3);
    }
    let mut off = sampler(vec![zone(dc(0.5, 960), Zone::default())]);
    off.set_parameter(0, 0.49);
    assert_eq!(
        play(&mut off, 60, 1.0, 4),
        [0.0; 4],
        "Enable below 0.5 is off"
    );
    off.set_parameter(0, 0.5);
    assert_eq!(play(&mut off, 60, 1.0, 4)[2], 0.5);
}

#[test]
fn reading_interpolates_linearly_between_frames() {
    // At half the device rate the root key reads half a frame per frame.
    let ramp = sample(24_000, (0..1000).map(|n| n as f32 / 1000.0).collect());
    let mut dw = sampler(vec![zone(ramp, Zone::default())]);
    let out = play(&mut dw, 48, 1.0, 8);
    close(f64::from(out[1]), 0.0005, 1e-4);
    close(f64::from(out[3]), 0.0015, 1e-4);
    assert_eq!(out[2], 0.001);
}

#[test]
fn the_loop_end_is_clipped_to_the_sample() {
    let past = Zone {
        loop_mode: 2,
        loop_start: 200,
        loop_end: 5000,
        ..Zone::default()
    };
    let out = play(&mut sampler(vec![zone(ramp(1000), past)]), 48, 1.0, 1600);
    assert_eq!(out[999], 0.999);
    assert_eq!(out[1000], 0.2);
    assert_eq!(out[1500], 0.7);
}

#[test]
fn a_zone_gain_is_held_to_16() {
    let loud = Zone {
        gain: 100.0,
        ..Zone::default()
    };
    assert_eq!(
        play(&mut sampler(vec![zone(dc(0.01, 960), loud)]), 60, 1.0, 4)[2],
        0.16
    );
}

#[test]
fn notes_are_ignored_while_enable_is_off() {
    let mut dw = sampler(vec![zone(dc(0.5, 9600), Zone::default())]);
    dw.set_parameter(0, 0.0);
    dw.note_on(60, 1.0, 0.5);
    dw.set_parameter(0, 1.0);
    assert_eq!(render(&mut dw, 64).0, [0.0; 64]);
}

#[test]
fn a_mono_retrigger_kills_the_old_voice_in_1_ms() {
    let zones = vec![
        looping_dc(0.25, keys(0, 59)),
        looping_dc(0.5, keys(60, 128)),
    ];
    let mut dw = sampler(zones);
    dw.set_parameter(11, 1.0);
    // Release 1: 10 s, so without the kill the old voice would ring on.
    dw.set_parameter(2, 1.0);
    play(&mut dw, 48, 1.0, 100);
    dw.note_off(48);
    render(&mut dw, 10);
    let out = play(&mut dw, 72, 1.0, 100);
    // The old voice falls by 1/48 per sample from about 1: half way at 24 samples, gone by 48.
    close(f64::from(out[24]), 0.5 + 0.25 * 0.25, 0.01);
    assert_eq!(out[50..], [0.5; 50]);
}

#[test]
fn mono_returns_to_the_newest_held_key() {
    let semitones = |st: f64| 441.0 * 2f64.powf(st / 12.0);
    let held = |release: &[u8]| {
        let mut dw = piano();
        dw.set_parameter(11, 1.0);
        for key in [48, 52, 55] {
            play(&mut dw, key, 1.0, 960);
        }
        for &key in release {
            dw.note_off(key);
        }
        pitch(&render(&mut dw, 4800).0[480..])
    };
    close(held(&[]), semitones(7.0), 1e-4);
    // Releasing the sounding key goes back to the newest key still held, not the oldest.
    close(held(&[55]), semitones(4.0), 1e-4);
    close(held(&[55, 52]), 441.0, 1e-4);
    // Releasing a key that is not sounding changes nothing.
    close(held(&[52]), semitones(7.0), 1e-4);
    close(held(&[52, 55]), 441.0, 1e-4);
}

#[test]
fn a_mono_hand_over_glides_from_the_sounding_key() {
    let mut dw = piano();
    dw.set_parameter(11, 1.0);
    // Slide 0.5: t = (1/4)³ s, about 16 ms.
    dw.set_parameter(7, 0.5);
    play(&mut dw, 48, 1.0, 4800);
    let up = play(&mut dw, 60, 1.0, 9600);
    let early = pitch(&up[..480]);
    assert!(early > 441.0 && early < 800.0, "{early}");
    close(pitch(&up[4800..]), 882.0, 1e-3);
    dw.note_off(60);
    let down = render(&mut dw, 9600).0;
    let early = pitch(&down[..480]);
    assert!(early > 480.0 && early < 882.0, "{early}");
    close(pitch(&down[4800..]), 441.0, 1e-3);
}

/// A held looping DC 0.5 note with the LFO set by `set`, after `frames`; the left and right channels.
fn lfo_note(set: &[(usize, f32)], frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut dw = sampler(vec![looping_dc(0.5, Zone::default())]);
    for &(index, value) in set {
        dw.set_parameter(index, value);
    }
    dw.note_on(48, 1.0, 0.5);
    render(&mut dw, frames)
}

// LFO params: 27 shape (Sine, Saw, Square, Random), 28 rate, 29 mode (Hertz, One Shot, Tmp Sync), 31 amount,
// 32 destination (Filter, Frequency, Level, Pan). Rate 0 is 0.03 Hz: the phase barely moves for the first seconds.
const SQUARE: (usize, f32) = (27, 2.0 / 3.0);

#[test]
fn the_level_destination_scales_by_1_plus_l() {
    // Square at φ = 0 is −1; amount 0.75 is u = 0.5, depth u|u| = 0.25: l = −0.25, gain 0.75.
    let (left, _) = lfo_note(&[SQUARE, (31, 0.75), (32, 2.0 / 3.0)], 4800);
    close(f64::from(left[4000]), 0.5 * 0.75, 1e-4);
    // Amount 1, l = −1: the gain heads to 0 by 0.01 of the gap per sample.
    let (left, _) = lfo_note(&[SQUARE, (31, 1.0), (32, 2.0 / 3.0)], 200);
    close(f64::from(left[0]), 0.5 * 0.99, 1e-5);
    close(f64::from(left[99]), 0.5 * 0.99f64.powi(100), 1e-3);
}

#[test]
fn the_pan_destination_turns_one_side_down() {
    // l = −0.25: L = min(1, 1 + l) = 0.75, R = min(1, 1 − l) = 1.
    let (left, right) = lfo_note(&[SQUARE, (31, 0.75), (32, 1.0)], 4800);
    close(f64::from(left[4000]), 0.5 * 0.75, 1e-4);
    close(f64::from(right[4000]), 0.5, 1e-6);
    // l = +0.25 (amount 0.25): L = min(1, 1.25) = 1, R = 0.75.
    let (left, right) = lfo_note(&[SQUARE, (31, 0.25), (32, 1.0)], 4800);
    close(f64::from(left[4000]), 0.5, 1e-6);
    close(f64::from(right[4000]), 0.5 * 0.75, 1e-4);
}

#[test]
fn the_frequency_destination_bends_by_l_times_its_magnitude() {
    // One Shot saw at 30 Hz: after 1/30 s φ holds at 1, saw = −1; amount 0.75 gives l = −0.25, −1/16 octave.
    let mut dw = piano();
    for (index, value) in [
        (27, 1.0 / 3.0),
        (28, 1.0),
        (29, 0.5),
        (31, 0.75),
        (32, 1.0 / 3.0),
    ] {
        dw.set_parameter(index, value);
    }
    let out = play(&mut dw, 48, 1.0, 48_000);
    close(pitch(&out[4800..]), 441.0 * 2f64.powf(-1.0 / 16.0), 1e-4);
}

#[test]
fn the_filter_destination_moves_the_cutoff_by_l() {
    // Lowpass at 0.9 (about 10 kHz) passes 5 kHz; a square at −1 on Filter pulls the cutoff to 0: 16 Hz.
    let rms = |set: &[(usize, f32)]| {
        let mut dw = sampler(vec![zone(sine(5000.0, 48_000, 1.0), Zone::default())]);
        for &(index, value) in [(16, 1.0 / 3.0), (12, 0.9)].iter().chain(set) {
            dw.set_parameter(index, value);
        }
        let out = play(&mut dw, 48, 1.0, 9600);
        (out[4800..].iter().map(|s| s * s).sum::<f32>() / 4800.0).sqrt()
    };
    let open = rms(&[]);
    assert!(open > 0.5, "{open}");
    assert!(rms(&[SQUARE, (31, 1.0), (32, 0.0)]) < 0.001 * open);
}

#[test]
fn the_frequency_envelope_cubes_e() {
    // Attack 0, Decay instant to Sustain 0.5: e = 0.5, ratio 2^(4·1·0.125) = √2 at amount 1.
    let mut dw = sampler(vec![zone(sine(200.0, 48_000, 4.0), Zone::default())]);
    for (index, value) in [(23, 0.0), (24, 0.5), (26, 1.0)] {
        dw.set_parameter(index, value);
    }
    close(
        pitch(&play(&mut dw, 48, 1.0, 9600)[480..]),
        200.0 * 2f64.sqrt(),
        1e-3,
    );
}

/// The RMS of a quiet 1 kHz tone through filter type `filter` with its cutoff at 1 kHz and resonance `res`.
fn at_cutoff(filter: f32, res: f32) -> f32 {
    let quiet = sample(
        48_000,
        sine(1000.0, 48_000, 1.0)
            .left
            .iter()
            .map(|s| s * 0.05)
            .collect(),
    );
    let mut dw = sampler(vec![zone(quiet, Zone::default())]);
    dw.set_parameter(16, filter);
    dw.set_parameter(12, (1000f32.log2() - 4.0) / 10.4);
    dw.set_parameter(13, res);
    let out = play(&mut dw, 48, 1.0, 24_000);
    (out[12_000..].iter().map(|s| s * s).sum::<f32>() / 12_000.0).sqrt()
}

#[test]
fn ladder_feedback_is_4_res() {
    // A 4-pole ladder at its cutoff: H = −1/4 open loop, so k = 4·Res gives |H| = 0.25/(1 − Res): ×2 at 0.5, ×4 at 0.75.
    let base = at_cutoff(1.0 / 3.0, 0.0);
    close(f64::from(base), 0.05 * 0.25 / 2f64.sqrt(), 0.02);
    close(f64::from(at_cutoff(1.0 / 3.0, 0.5) / base), 2.0, 0.03);
    close(f64::from(at_cutoff(1.0 / 3.0, 0.75) / base), 4.0, 0.03);
}

#[test]
fn bandpass_q_is_16_res_cubed_plus_half() {
    // Constant skirt gain: the peak gain is Q. Res 0.5: Q = 2.5.
    let dry = 0.05 / 2f32.sqrt();
    close(f64::from(at_cutoff(2.0 / 3.0, 0.5) / dry), 2.5, 0.02);
    close(f64::from(at_cutoff(2.0 / 3.0, 0.0) / dry), 0.5, 0.02);
    // The highpass output x − k·band − low has the same gain Q at the cutoff.
    close(f64::from(at_cutoff(1.0, 0.5) / dry), 2.5, 0.02);
    close(f64::from(at_cutoff(1.0, 0.0) / dry), 0.5, 0.02);
}

#[test]
fn the_filter_type_truncates() {
    // Filter type is trunc(3v). Just below Lowpass's step it is still Off; at f32(1/3) it is Lowpass.
    let rms = |v: f32| {
        let mut dw = sampler(vec![zone(sine(5000.0, 48_000, 1.0), Zone::default())]);
        dw.set_parameter(16, v);
        dw.set_parameter(12, 0.4775);
        let out = play(&mut dw, 48, 1.0, 9600);
        (out[4800..].iter().map(|s| s * s).sum::<f32>() / 4800.0).sqrt()
    };
    assert!(rms(0.333) > 0.7, "0.333 is Off");
    assert!(rms(1.0 / 3.0) < 0.01, "f32(1/3) is Lowpass");
}

#[test]
fn loops_wrap_by_their_overshoot_and_interpolate_across_the_seam() {
    let looping = Zone {
        loop_mode: 2,
        loop_start: 200,
        loop_end: 400,
        ..Zone::default()
    };
    let ramp_at = |rate: u32| sample(rate, (0..1000).map(|n| n as f32 / 1000.0).collect());
    // 1.5 frames a frame: 399 is followed by 400.5, which wraps to 200.5, not 200.
    let out = play(
        &mut sampler(vec![zone(ramp_at(72_000), looping)]),
        48,
        1.0,
        300,
    );
    assert_eq!(out[266], 0.399);
    close(f64::from(out[267]), 0.2005, 1e-4);
    // Half a frame a frame: 399.5 reads between 399 and the loop start.
    let out = play(
        &mut sampler(vec![zone(ramp_at(24_000), looping)]),
        48,
        1.0,
        900,
    );
    close(f64::from(out[799]), (0.399 + 0.2) / 2.0, 1e-4);
    // Reverse ignores the loop, between frames too: 399.5 reads 399 and 400.
    let mut reverse = sampler(vec![zone(ramp_at(24_000), looping)]);
    reverse.set_parameter(8, 1.0);
    let out = play(&mut reverse, 48, 1.0, 1300);
    close(f64::from(out[1199]), 0.3995, 1e-4);
}

#[test]
fn the_pb_range_setting_applies_live() {
    // The panel's PB Range (setting #11800, k/12) reaches the sampler without a rebuild.
    let mut dw = piano();
    dw.set_controller(4, 1.0);
    dw.set_setting(11_800, 7.0 / 12.0);
    close(
        pitch(&play(&mut dw, 48, 1.0, 9600)[480..]),
        441.0 * 2f64.powf(7.0 / 12.0),
        1e-4,
    );
    // Other keys are not the DW's.
    dw.set_setting(3100, 1.0);
    close(
        pitch(&play(&mut dw, 48, 1.0, 9600)[480..]),
        441.0 * 2f64.powf(7.0 / 12.0),
        1e-4,
    );
}

/// Slides add to every voice's pitch, velocity and note pan; a voice launched later
/// starts without them.
#[test]
fn slides_shift_the_pitch_velocity_and_pan_of_every_voice() {
    let piano = || vec![zone(sine(441.0, 44_100, 2.0), keys(0, 127))];
    let mut dw = sampler(piano());
    dw.note_on(48, 1.0, 0.5);
    render(&mut dw, 2400);
    dw.set_slide(12.0, 0.0, 0.0);
    close(pitch(&render(&mut dw, 9600).0), 882.0, 1e-4);
    // The slid voice is silenced (velocity − 1), and a second voice on the same key plays unshifted.
    dw.set_slide(12.0, 0.0, -1.0);
    dw.note_on(48, 1.0, 0.5);
    render(&mut dw, 2400);
    close(pitch(&render(&mut dw, 9600).0), 441.0, 1e-4);

    // A DC zone makes the gains exact.
    let flat = |slide: [f32; 3]| {
        let mut dw = sampler(vec![zone(dc(0.5, 96_000), keys(0, 127))]);
        dw.note_on(48, 1.0, 0.5);
        render(&mut dw, 4800);
        dw.set_slide(slide[0], slide[1], slide[2]);
        let (left, right) = render(&mut dw, 480);
        (f64::from(left[479]), f64::from(right[479]))
    };
    let (left, right) = flat([0.0; 3]);
    assert!(left > 0.01, "{left}");
    close(right, left, 1e-6);
    let (quieter, _) = flat([0.0, 0.0, -0.5]);
    close(quieter, 0.5 * left, 1e-6);
    // p = clamp(0.5 + 0.5, 0, 1) = 1: L = min(1, 2 − 2p) = 0, R = min(1, 2p) = 1.
    let (hard_left, hard_right) = flat([0.0, 0.5, 0.0]);
    assert_eq!(hard_left, 0.0);
    close(hard_right, right, 1e-6);
}
