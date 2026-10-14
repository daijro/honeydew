//! The pad sampler, rendered offline.

use std::f32::consts::TAU;
use std::sync::Arc;

use honeydew_audio::drums::{
    EG_A, EG_D, EG_R, EG_S, LENGTH, LEVEL, MUTE, PAN, Pad, PadSample, PadSampler, SAMPLE_START,
    SOLO, SPEED, STREAM_FRAMES, Stream, Stretching, hit_pitch, level_gain, stretch,
};
use honeydew_audio::sample::SampleData;

const RATE: f32 = 44_100.0;

fn mono(samples: Vec<f32>, rate: u32) -> SampleData {
    SampleData {
        sample_rate: rate,
        left: samples.into(),
        right: None,
        ..Default::default()
    }
}

/// A constant 1.0 of `frames` frames, so the output traces the gains and the envelope.
fn dc(frames: usize) -> SampleData {
    mono(vec![1.0; frames], RATE as u32)
}

/// `x[i] = i`, so the output reads back the play position.
fn ramp(frames: usize, rate: u32) -> SampleData {
    mono((0..frames).map(|i| i as f32).collect(), rate)
}

fn sine(hz: f32, frames: usize) -> SampleData {
    mono(
        (0..frames)
            .map(|i| (TAU * hz * i as f32 / RATE).sin())
            .collect(),
        RATE as u32,
    )
}

fn pad(sample: SampleData) -> Pad {
    Pad {
        sample: Some(PadSample::Whole(Arc::new(sample))),
        ..Pad::default()
    }
}

/// `frames` frames of pad `index`, rendered in 64-frame blocks.
fn render(sampler: &mut PadSampler, index: usize, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    for (l, r) in left.chunks_mut(64).zip(right.chunks_mut(64)) {
        sampler.process(index, l, r);
    }
    (left, right)
}

fn left(sampler: &mut PadSampler, index: usize, frames: usize) -> Vec<f32> {
    render(sampler, index, frames).0
}

/// The mean period in frames, from the rising zero crossings.
fn period(samples: &[f32]) -> f32 {
    let rising: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .collect();
    (rising[rising.len() - 1] - rising[0]) as f32 / (rising.len() - 1) as f32
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn the_level_law_is_66_v_to_the_0_6_minus_60_db() {
    assert!(close(level_gain(0.8531), 1.0, 1e-3), "the default is unity");
    assert!(close(level_gain(0.0), 0.001, 1e-7), "v = 0 plays at -60 dB");
    assert!(close(level_gain(1.0), 10f32.powf(0.3), 1e-5));
    let db = |v: f32| 20.0 * level_gain(v).log10();
    for (v, want) in [
        (0.1, -43.4),
        (0.25, -31.3),
        (0.5, -16.5),
        (0.75, -4.5),
        (0.9, 2.0),
    ] {
        assert!(close(db(v), want, 0.05), "{v}: {}", db(v));
    }
}

#[test]
fn hit_pitch_spans_an_octave_each_way() {
    assert_eq!(hit_pitch(0), 0.5);
    assert!(close(hit_pitch(0x7FFF), 1.0, 1e-4));
    assert_eq!(hit_pitch(65535), 2.0);
}

#[test]
fn a_default_pad_plays_its_whole_sample_at_unity() {
    let mut sampler = PadSampler::new(vec![pad(dc(1000))], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let (l, r) = render(&mut sampler, 0, 1200);
    assert!(
        l[..1000]
            .iter()
            .chain(&r[..1000])
            .all(|&s| close(s, 1.0, 1e-3))
    );
    assert!(
        l[1000..].iter().all(|&s| s == 0.0),
        "the voice ends past the last frame"
    );
    assert_eq!(sampler.active_voices(0), 0);
}

#[test]
fn hit_velocity_and_pan_then_slot_level_and_pan() {
    let mut sampler = PadSampler::new(vec![pad(dc(100))], RATE);
    sampler.hit(0, 0.5, 0.25, 1.0);
    let (l, r) = render(&mut sampler, 0, 10);
    assert!(
        close(l[5], 0.5, 1e-3) && close(r[5], 0.25, 1e-3),
        "{} {}",
        l[5],
        r[5]
    );

    // Pad pan 0.75: the pan law halves the left side. Level 0.5 is -16.5 dB.
    let mut sampler = PadSampler::new(vec![pad(dc(100))], RATE);
    sampler.set_param(0, PAN, 0.75);
    sampler.set_param(0, LEVEL, 0.5);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let (l, r) = render(&mut sampler, 0, 10);
    let g = level_gain(0.5);
    assert!(close(l[5], 0.5 * g, 1e-5) && close(r[5], g, 1e-5));
}

#[test]
fn speed_bend_and_hit_pitch_scale_the_step() {
    let base = || PadSampler::new(vec![pad(sine(441.0, 44_100))], RATE);
    let mut plain = base();
    plain.hit(0, 1.0, 0.5, 1.0);
    assert!(close(period(&left(&mut plain, 0, 4000)), 100.0, 0.1));

    let mut speed = base();
    speed.set_param(0, SPEED, 36.0 / 48.0); // +12 semitones
    speed.hit(0, 1.0, 0.5, 1.0);
    assert!(close(period(&left(&mut speed, 0, 4000)), 50.0, 0.1));

    let mut down = base();
    down.set_param(0, SPEED, 12.0 / 48.0); // -12 semitones
    down.hit(0, 1.0, 0.5, 1.0);
    assert!(close(period(&left(&mut down, 0, 4000)), 200.0, 0.2));

    let mut bend = base();
    bend.set_bend(0.75); // w = 1: one octave up
    bend.hit(0, 1.0, 0.5, 1.0);
    assert!(close(period(&left(&mut bend, 0, 4000)), 50.0, 0.1));

    let mut pitched = base();
    pitched.hit(0, 1.0, 0.5, hit_pitch(65535));
    assert!(close(period(&left(&mut pitched, 0, 4000)), 50.0, 0.1));
}

#[test]
fn the_step_converts_the_sample_rate_with_linear_interpolation() {
    // A 44.1 kHz ramp at 88.2 kHz: half a source frame per output frame, interpolated linearly.
    let mut sampler = PadSampler::new(vec![pad(ramp(100, 44_100))], 88_200.0);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let l = left(&mut sampler, 0, 300);
    let gain = level_gain(0.8531);
    for (i, s) in l[..199].iter().enumerate() {
        assert!(close(*s, i as f32 * 0.5 * gain, 1e-3), "{i}: {s}");
    }
    assert!(l[199..].iter().all(|&s| s == 0.0), "ends past frame 99");
}

#[test]
fn reverse_plays_from_the_end() {
    let mut sampler = PadSampler::new(
        vec![Pad {
            reverse: true,
            ..pad(ramp(100, RATE as u32))
        }],
        RATE,
    );
    sampler.hit(0, 1.0, 0.5, 1.0);
    let l = left(&mut sampler, 0, 120);
    let gain = level_gain(0.8531);
    for (i, s) in l[..100].iter().enumerate() {
        assert!(close(*s, (99 - i) as f32 * gain, 1e-3), "{i}: {s}");
    }
}

#[test]
fn sample_start_skips_or_inserts_source_time() {
    // v = 0.75: u = 0.5, s = +0.125 s; v = 0.25: s = -0.125 s. At 8 kHz that is 1000 frames.
    let rate = 8000;
    let start = |v: f32, reverse: bool| {
        let mut sampler = PadSampler::new(
            vec![Pad {
                reverse,
                ..pad(ramp(4000, rate))
            }],
            rate as f32,
        );
        sampler.set_param(0, SAMPLE_START, v);
        sampler.hit(0, 1.0, 0.5, 1.0);
        let gain = level_gain(0.8531);
        left(&mut sampler, 0, 6000)
            .into_iter()
            .map(|s| (s / gain).round())
            .collect::<Vec<_>>()
    };
    let skip = start(0.75, false);
    assert_eq!(skip[0], 1000.0, "positive: sounds at once from 0.125 s in");
    assert_eq!(skip[2999], 3999.0);
    assert_eq!(skip[3000], 0.0);
    let delay = start(0.25, false);
    assert!(
        delay[..1000].iter().all(|&s| s == 0.0),
        "negative: silence first"
    );
    assert_eq!(delay[1001], 1.0);
    assert_eq!(delay[4999], 3999.0);
    let reversed = start(0.75, true);
    assert_eq!(reversed[0], 2999.0, "reversed: the skip comes off the end");
}

/// Frames until a constant sample's output first reaches `level` of full scale.
fn frames_to(output: &[f32], level: f32) -> usize {
    output.iter().position(|&s| s >= level).unwrap()
}

#[test]
fn attack_decay_and_release_follow_the_coefficient_law() {
    let gain = level_gain(0.8531);
    // Attack v = 0.1: full after ln 0.8 / ln(1 - c_A) = 246.5 frames (5.6 ms at 44.1 kHz).
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.set_param(0, EG_A, 0.1);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let attack = left(&mut sampler, 0, 1000);
    assert!(
        (245..=248).contains(&frames_to(&attack, 0.9999 * gain)),
        "{}",
        frames_to(&attack, 0.9999 * gain)
    );

    // Decay v = 0.25 to S = 0.5: tau 38.7 ms = 1707 frames.
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.set_param(0, EG_D, 0.25);
    sampler.set_param(0, EG_S, 0.5);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let decay = left(&mut sampler, 0, 20_000);
    let tau = frames_to(
        &decay.iter().map(|s| -s).collect::<Vec<_>>(),
        -(0.5 + 0.5 / 1f32.exp()) * gain,
    );
    assert!((1690..=1725).contains(&tau), "decay tau {tau}");
    assert!(close(decay[19_999], 0.5 * gain, 1e-3));

    // Release v = 0.25 after a note-off: tau 1707 frames, then the voice ends below 1e-4.
    let mut sampler = PadSampler::new(vec![pad(dc(1_000_000))], RATE);
    sampler.set_param(0, EG_R, 0.25);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 100);
    sampler.release(0);
    let release = left(&mut sampler, 0, 40_000);
    let tau = frames_to(
        &release.iter().map(|s| -s).collect::<Vec<_>>(),
        -gain / 1f32.exp(),
    );
    assert!((1690..=1725).contains(&tau), "release tau {tau}");
    assert_eq!(sampler.active_voices(0), 0);
}

#[test]
fn the_length_gate_starts_the_release_after_n_times_length_frames() {
    // EG R = 0 releases at once, so the gate cuts the 1000-frame sample after 500 frames.
    let mut sampler = PadSampler::new(vec![pad(dc(1000))], RATE);
    sampler.set_param(0, LENGTH, 0.5);
    sampler.set_param(0, EG_R, 0.0);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let l = left(&mut sampler, 0, 1000);
    assert!(l[..500].iter().all(|&s| s > 0.99));
    assert!(l[500..].iter().all(|&s| s == 0.0));
}

#[test]
fn overlap_off_fades_the_pads_own_voice_on_a_retrigger() {
    let twice = |overlap: bool| {
        let mut sampler = PadSampler::new(
            vec![Pad {
                overlap,
                ..pad(dc(100_000))
            }],
            RATE,
        );
        sampler.hit(0, 1.0, 0.5, 1.0);
        left(&mut sampler, 0, 100);
        sampler.hit(0, 1.0, 0.5, 1.0);
        left(&mut sampler, 0, 3000)
    };
    let gain = level_gain(0.8531);
    let overlapping = twice(true);
    assert!(close(overlapping[2999], 2.0 * gain, 1e-3));
    let cut = twice(false);
    // The fast fade (c = 0.01 per frame) is not a hard cut: the old voice still sounds right after the retrigger.
    assert!(
        cut[0] > 1.9 * gain && close(cut[0], (1.0 + 0.99) * gain, 1e-3),
        "{}",
        cut[0]
    );
    assert!(close(cut[2999], gain, 1e-4));
}

#[test]
fn thirty_two_voices_per_pad_then_the_furthest_is_stolen() {
    let mut sampler = PadSampler::new(vec![pad(dc(1000))], RATE);
    let gain = level_gain(0.8531);
    let mut out = Vec::new();
    for _ in 0..33 {
        sampler.hit(0, 1.0, 0.5, 1.0);
        out.extend(left(&mut sampler, 0, 10));
    }
    assert_eq!(sampler.active_voices(0), 32);
    assert!(close(out[329], 32.0 * gain, 1e-2));
    // The 33rd hit took the first voice's place, so 32 still sound when that one would have ended (frame 1000).
    out.extend(left(&mut sampler, 0, 700));
    assert!(close(out[1005], 32.0 * gain, 1e-2), "{}", out[1005]);
}

#[test]
fn a_cut_group_hit_or_note_off_fast_fades_the_groups_other_pads() {
    let hats = || {
        let group = |cut_group| Pad {
            cut_group,
            ..pad(dc(100_000))
        };
        PadSampler::new(vec![group(1), group(1), group(0)], RATE)
    };
    let mut sampler = hats();
    sampler.hit(0, 1.0, 0.5, 1.0);
    sampler.hit(2, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 100);
    left(&mut sampler, 2, 100);
    sampler.hit(1, 1.0, 0.5, 1.0);
    let open = left(&mut sampler, 0, 2000);
    assert!(
        open[0] > 0.98 && open[0] < 1.0,
        "a fade, not a cut: {}",
        open[0]
    );
    assert!(
        open[1500..].iter().all(|&s| s == 0.0),
        "about 21 ms to -80 dB"
    );
    assert_eq!(sampler.active_voices(0), 0);
    assert_eq!(
        sampler.active_voices(1),
        1,
        "the pad that hit keeps its voice"
    );
    assert_eq!(sampler.active_voices(2), 1, "group 0 chokes nothing");

    let mut sampler = hats();
    sampler.hit(0, 1.0, 0.5, 1.0);
    sampler.release(1);
    left(&mut sampler, 0, 2000);
    assert_eq!(sampler.active_voices(0), 0, "a note-off chokes too");
}

#[test]
fn mute_and_solo_silence_pads_whose_voices_keep_running() {
    let mut sampler = PadSampler::new(vec![pad(ramp(1000, RATE as u32)), pad(dc(1000))], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    sampler.set_param(0, MUTE, 1.0);
    assert!(left(&mut sampler, 0, 100).iter().all(|&s| s == 0.0));
    sampler.set_param(0, MUTE, 0.0);
    let gain = level_gain(0.8531);
    assert!(
        close(left(&mut sampler, 0, 1)[0], 100.0 * gain, 1e-2),
        "it ran on muted"
    );

    sampler.hit(1, 1.0, 0.5, 1.0);
    sampler.set_param(1, SOLO, 1.0);
    assert!(
        left(&mut sampler, 0, 10).iter().all(|&s| s == 0.0),
        "another pad's solo"
    );
    assert!(left(&mut sampler, 1, 10).iter().all(|&s| s > 0.99));
}

#[test]
fn live_notes_play_every_pad_on_that_key_centred() {
    let on = |note| Pad {
        note,
        ..pad(dc(100_000))
    };
    let mut sampler = PadSampler::new(vec![on(Some(48)), on(Some(48)), on(Some(50))], RATE);
    sampler.note_on(48, 0.5);
    for (index, voices) in [1, 1, 0].into_iter().enumerate() {
        assert_eq!(sampler.active_voices(index), voices);
    }
    let (l, r) = render(&mut sampler, 0, 10);
    assert!(
        close(l[5], 0.5, 1e-3) && close(r[5], 0.5, 1e-3),
        "pan centred"
    );
    sampler.set_param(1, EG_R, 0.0);
    sampler.note_off(48);
    left(&mut sampler, 1, 10);
    assert_eq!(sampler.active_voices(1), 0, "released");
}

#[test]
fn all_notes_off_releases_and_reset_silences() {
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.set_param(0, EG_R, 0.1);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 10);
    sampler.all_notes_off();
    let l = left(&mut sampler, 0, 2000);
    assert!(l[0] > 0.9, "a release, not a cut");
    assert_eq!(sampler.active_voices(0), 0);

    sampler.hit(0, 1.0, 0.5, 1.0);
    sampler.reset();
    assert!(left(&mut sampler, 0, 10).iter().all(|&s| s == 0.0));
}

#[test]
fn a_stereo_sample_plays_in_stereo() {
    let sample = SampleData {
        sample_rate: RATE as u32,
        left: vec![1.0; 100].into(),
        right: Some(vec![-0.5; 100].into()),
        ..Default::default()
    };
    let mut sampler = PadSampler::new(vec![pad(sample)], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let (l, r) = render(&mut sampler, 0, 10);
    let gain = level_gain(0.8531);
    assert!(close(l[5], gain, 1e-5) && close(r[5], -0.5 * gain, 1e-5));
}

#[test]
fn block_size_does_not_change_the_output() {
    let play = |blocks: &[usize]| {
        let mut sampler = PadSampler::new(vec![pad(sine(300.0, 20_000))], RATE);
        sampler.set_param(0, EG_A, 0.2);
        sampler.set_param(0, LENGTH, 0.3);
        sampler.set_param(0, EG_R, 0.2);
        sampler.hit(0, 0.8, 0.3, 1.1);
        let mut out = Vec::new();
        for &frames in blocks.iter().cycle() {
            if out.len() >= 15_000 {
                break;
            }
            let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
            sampler.process(0, &mut l, &mut r);
            out.extend(l.into_iter().zip(r));
        }
        out.truncate(15_000);
        out
    };
    assert_eq!(play(&[15_000]), play(&[1, 7, 333, 1023, 64]));
}

#[test]
fn stretch_renders_length_and_pitch_once() {
    let source = sine(441.0, 44_100);
    let same = stretch(&source, 1.0, 0.0);
    assert_eq!(same.left, source.left, "no change is the identity");

    let long = stretch(&source, 2.0, 0.0);
    assert_eq!(long.frames(), 88_200);
    assert!(
        close(period(&long.left[20_000..60_000]), 100.0, 0.5),
        "length only"
    );

    let up = stretch(&source, 1.0, 12.0);
    assert_eq!(up.frames(), 44_100);
    assert!(
        close(period(&up.left[5000..30_000]), 50.0, 0.5),
        "pitch only"
    );
    assert_eq!(stretch(&source, 1.0, 11.6).left, up.left, "whole semitones");
}

#[test]
fn stretch_rescales_the_loop_and_cues_into_the_new_length() {
    let mut source = sine(441.0, 1000);
    source.meta.loop_frames = Some((100, 900));
    source.meta.cues = vec![0, 250, 999, 5000];
    let long = stretch(&source, 2.0, 0.0);
    assert_eq!(long.frames(), 2000);
    assert_eq!(long.meta.loop_frames, Some((200, 1800)));
    assert_eq!(
        long.meta.cues,
        [0, 500, 1998, 2000],
        "past the end: held to it"
    );
    let short = stretch(&source, 0.5, 0.0);
    assert_eq!(short.meta.loop_frames, Some((50, 450)));
    let pitched = stretch(&source, 1.0, 7.0).meta;
    assert_eq!(pitched.loop_frames, source.meta.loop_frames, "pitch alone");
    assert_eq!(pitched.cues, [0, 250, 999, 1000]);
}

#[test]
fn stretch_fits_a_loop_that_runs_past_the_end_to_the_new_length() {
    // The loop is held to the rendered length as at load: an end past it becomes the length, a start at or past it 0.
    let mut source = sine(441.0, 1000);
    source.meta.loop_frames = Some((100, 5000));
    let long = stretch(&source, 2.0, 0.0);
    assert_eq!(long.frames(), 2000);
    assert_eq!(
        long.meta.loop_frames,
        Some((200, 2000)),
        "10000 held to 2000"
    );
    source.meta.loop_frames = Some((1200, 1500));
    let short = stretch(&source, 0.5, 0.0);
    assert_eq!(short.frames(), 500);
    assert_eq!(
        short.meta.loop_frames,
        Some((0, 500)),
        "600 is past the end"
    );
}

#[test]
fn stretch_rescales_by_the_length_it_rendered_not_the_ratio_asked() {
    // 1000 frames at x1.2347 render 1235, so the loop and cues scale by 1.235: 999 goes to 1234 (the asked ratio would
    // give 1233) and 777 to 960 (959).
    let mut source = sine(441.0, 1000);
    source.meta.loop_frames = Some((777, 999));
    source.meta.cues = vec![0, 999];
    let long = stretch(&source, 1.2347, 0.0);
    assert_eq!(long.frames(), 1235);
    assert_eq!(long.meta.loop_frames, Some((960, 1234)));
    assert_eq!(long.meta.cues, [0, 1234]);
}

#[test]
fn hit_at_adds_a_source_time_offset_that_speed_does_not_scale() {
    // At 8 kHz, 0.125 s is 1000 frames. Audio clips start at (Sample Start + offset) seconds.
    let rate = 8000;
    let first = |sample_start: f32, speed: f32, offset: f64| {
        let mut sampler = PadSampler::new(vec![pad(ramp(8000, rate))], rate as f32);
        sampler.set_param(0, SAMPLE_START, sample_start);
        sampler.set_param(0, SPEED, speed);
        sampler.hit_at(0, 1.0, 0.5, 1.0, offset);
        let gain = level_gain(0.8531);
        left(&mut sampler, 0, 3000)
            .into_iter()
            .map(|s| (s / gain).round())
            .collect::<Vec<_>>()
    };
    assert_eq!(first(0.5, 0.5, 0.0)[0], 0.0);
    assert_eq!(first(0.5, 0.5, 0.125)[0], 1000.0);
    assert_eq!(first(0.75, 0.5, 0.125)[0], 2000.0, "added to Sample Start");
    let fast = first(0.5, 0.75, 0.125);
    assert_eq!(
        (fast[0], fast[1]),
        (1000.0, 1002.0),
        "Speed +12 steps 2, the offset stays"
    );
    let late = first(0.5, 0.5, -0.125);
    assert!(
        late[..1000].iter().all(|&s| s == 0.0),
        "negative: silence first"
    );
    assert_eq!(late[1001], 1.0);
}

#[test]
fn fade_fast_fades_only_that_pads_voices() {
    let mut sampler = PadSampler::new(vec![pad(dc(100_000)), pad(dc(100_000))], RATE);
    for index in 0..2 {
        sampler.hit(index, 1.0, 0.5, 1.0);
        left(&mut sampler, index, 10);
    }
    sampler.fade(0);
    let gain = level_gain(0.8531);
    let faded = left(&mut sampler, 0, 3000);
    assert!(
        close(faded[0], 0.99 * gain, 1e-4),
        "c = 0.01 per frame: {}",
        faded[0]
    );
    assert!(close(faded[99], 0.99f32.powi(100) * gain, 1e-4));
    assert_eq!(sampler.active_voices(0), 0, "ended below the silence floor");
    assert!(
        close(left(&mut sampler, 1, 10)[9], gain, 1e-4),
        "the other pad plays on"
    );
}

#[test]
fn speed_truncates_between_steps() {
    // trunc(48v − 24): 12.6 plays +12 and −12.6 plays −12, where rounding would give ±13.
    let period_at = |v: f32| {
        let mut sampler = PadSampler::new(vec![pad(sine(441.0, 44_100))], RATE);
        sampler.set_param(0, SPEED, v);
        sampler.hit(0, 1.0, 0.5, 1.0);
        period(&left(&mut sampler, 0, 8000))
    };
    assert!(close(period_at(36.6 / 48.0), 50.0, 0.1));
    assert!(close(period_at(11.4 / 48.0), 200.0, 0.2));
    assert!(close(period_at(24.9 / 48.0), 100.0, 0.1));
    assert!(close(period_at(23.1 / 48.0), 100.0, 0.1));
    // The fused law at the steps: k = 25 plays 0 semitones and k = 40 plays +15, not +1 and +16.
    assert!(close(period_at(25.0 / 48.0), 100.0, 0.1));
    let fifteen = 100.0 / (15.0f32 / 12.0).exp2();
    assert!(
        close(period_at(40.0 / 48.0), fifteen, 0.1),
        "{}",
        period_at(40.0 / 48.0)
    );
}

#[test]
fn mute_and_solo_are_on_only_above_one_half() {
    let mut sampler = PadSampler::new(vec![pad(dc(1000)), pad(dc(1000))], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    sampler.set_param(0, MUTE, 0.5);
    sampler.set_param(1, SOLO, 0.5);
    assert!(left(&mut sampler, 0, 10)[5] > 0.99, "0.5 is Off for both");
    sampler.set_param(0, MUTE, 0.5001);
    assert_eq!(left(&mut sampler, 0, 10)[5], 0.0);
    sampler.set_param(0, MUTE, 0.0);
    sampler.set_param(1, SOLO, 0.5001);
    assert_eq!(left(&mut sampler, 0, 10)[5], 0.0, "another pad's Solo");
    sampler.set_param(0, SOLO, 0.5);
    assert_eq!(
        left(&mut sampler, 0, 10)[5],
        0.0,
        "its own Solo at 0.5 is Off"
    );
    sampler.set_param(0, SOLO, 0.5001);
    assert!(left(&mut sampler, 0, 10)[5] > 0.99, "both soloed");
}

#[test]
fn both_sides_of_the_slot_and_hit_pan_laws() {
    let gains = |pad_pan: f32, hit_pan: f32| {
        let mut sampler = PadSampler::new(vec![pad(dc(100))], RATE);
        sampler.set_param(0, PAN, pad_pan);
        sampler.hit(0, 1.0, hit_pan, 1.0);
        let (l, r) = render(&mut sampler, 0, 10);
        let gain = level_gain(0.8531);
        (l[5] / gain, r[5] / gain)
    };
    let (l, r) = gains(0.25, 0.5);
    assert!(
        close(l, 1.0, 1e-5) && close(r, 0.5, 1e-5),
        "pad pan left: {l} {r}"
    );
    let (l, r) = gains(0.5, 0.75);
    assert!(
        close(l, 0.5, 1e-5) && close(r, 1.0, 1e-5),
        "hit pan right: {l} {r}"
    );
    let (l, r) = gains(0.5, 0.0);
    assert!(
        close(l, 1.0, 1e-5) && r == 0.0,
        "hit pan hard left: {l} {r}"
    );
}

#[test]
fn a_live_note_plays_at_the_pads_own_pitch() {
    let mut sampler = PadSampler::new(
        vec![Pad {
            note: Some(60),
            ..pad(sine(441.0, 44_100))
        }],
        RATE,
    );
    sampler.note_on(60, 1.0);
    assert!(close(period(&left(&mut sampler, 0, 4000)), 100.0, 0.1));
}

#[test]
fn all_notes_off_is_a_release_not_a_fade() {
    // EG R = 1: tau 5.67 s, so the voice still sounds long after a fast fade (about 917 frames) would have ended.
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 10);
    sampler.all_notes_off();
    assert!(left(&mut sampler, 0, 3000)[2999] > 0.95);
}

#[test]
fn a_fading_voice_keeps_fading_through_a_release() {
    let group = || Pad {
        cut_group: 1,
        ..pad(dc(100_000))
    };
    let mut sampler = PadSampler::new(vec![group(), group()], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 10);
    sampler.hit(1, 1.0, 0.5, 1.0);
    sampler.release(0); // EG R = 1 would take seconds
    left(&mut sampler, 0, 1000);
    assert_eq!(sampler.active_voices(0), 0);
}

#[test]
fn a_fast_fade_ends_below_1e_minus_4() {
    // 0.99^n < 1e-4 first at n = 917.
    let group = || Pad {
        cut_group: 2,
        ..pad(dc(100_000))
    };
    let mut sampler = PadSampler::new(vec![group(), group()], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 10);
    sampler.hit(1, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 910);
    assert_eq!(sampler.active_voices(0), 1, "still above 1e-4");
    left(&mut sampler, 0, 10);
    assert_eq!(sampler.active_voices(0), 0);
}

#[test]
fn only_a_voice_heading_for_zero_ends_when_quiet() {
    // A slow attack starts far below 1e-4 and keeps going.
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.set_param(0, EG_A, 1.0);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let l = left(&mut sampler, 0, 100);
    assert!(l[0] > 0.0 && l[0] < 1e-5 && l[99] > l[0]);
    assert_eq!(sampler.active_voices(0), 1);
    // A decay to S = 0 heads for zero and ends there.
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    sampler.set_param(0, EG_D, 0.1);
    sampler.set_param(0, EG_S, 0.0);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 2000);
    assert_eq!(sampler.active_voices(0), 0);
}

#[test]
fn d_and_r_are_instant_below_50_x4_of_0_001() {
    // x = 0.95v + 0.05: v = 0.015 gives 50x⁴ ≈ 0.00078, instant; v = 0.02 gives c ≈ 0.18, about 47 frames to 1e-4.
    let released = |v: f32, frames: usize| {
        let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
        sampler.set_param(0, EG_R, v);
        sampler.hit(0, 1.0, 0.5, 1.0);
        left(&mut sampler, 0, 10);
        sampler.release(0);
        left(&mut sampler, 0, frames);
        sampler.active_voices(0)
    };
    assert_eq!(released(0.015, 1), 0);
    assert_eq!(released(0.02, 10), 1);
    assert_eq!(released(0.02, 60), 0);
}

#[test]
fn a_hit_never_chokes_its_own_pad() {
    let mut sampler = PadSampler::new(
        vec![Pad {
            cut_group: 1,
            ..pad(dc(100_000))
        }],
        RATE,
    );
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 10);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let l = left(&mut sampler, 0, 2000);
    assert!(close(l[1999], 2.0 * level_gain(0.8531), 1e-3));
}

#[test]
fn a_hit_with_a_broken_value_plays_nothing() {
    let mut sampler = PadSampler::new(vec![pad(dc(100))], RATE);
    for (velocity, pan, pitch) in [
        (f32::NAN, 0.5, 1.0),
        (1.0, f32::NAN, 1.0),
        (1.0, 0.5, f32::NAN),
        (1.0, 0.5, 0.0),
        (1.0, 0.5, -1.0),
        (f32::INFINITY, 0.5, 1.0),
    ] {
        sampler.hit(0, velocity, pan, pitch);
        assert_eq!(sampler.active_voices(0), 0, "{velocity} {pan} {pitch}");
    }
}

#[test]
fn solo_follows_the_initial_params_and_every_write() {
    let soloed = Pad {
        params: {
            let mut params = Pad::default().params;
            params[SOLO] = 1.0;
            params
        },
        ..pad(dc(1000))
    };
    let mut sampler = PadSampler::new(vec![pad(dc(1000)), soloed], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    assert_eq!(left(&mut sampler, 0, 10)[5], 0.0, "soloed from the start");
    sampler.set_param(1, SOLO, 0.0);
    assert!(left(&mut sampler, 0, 10)[5] > 0.99, "unsoloed");
    sampler.set_param(1, SOLO, 1.0);
    sampler.set_param(1, LEVEL, 0.2);
    assert_eq!(left(&mut sampler, 0, 10)[5], 0.0, "soloed again");
}

#[test]
fn a_voice_keeps_the_direction_it_launched_with() {
    // `REV` re-renders nothing; each new voice takes the pad's
    // reverse when it launches, so a voice already playing keeps its direction.
    let mut sampler = PadSampler::new(vec![pad(ramp(100, RATE as u32))], RATE);
    let gain = level_gain(0.8531);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let mut heard = left(&mut sampler, 0, 10);
    sampler.set_reverse(0, true);
    heard.extend(left(&mut sampler, 0, 100));
    for (i, s) in heard[..100].iter().enumerate() {
        assert!(close(*s, i as f32 * gain, 1e-3), "forward {i}: {s}");
    }
    // The next voice plays from the end, and keeps doing so when reverse goes off again.
    sampler.hit(0, 1.0, 0.5, 1.0);
    let mut heard = left(&mut sampler, 0, 10);
    sampler.set_reverse(0, false);
    heard.extend(left(&mut sampler, 0, 90));
    for (i, s) in heard.iter().enumerate() {
        assert!(close(*s, (99 - i) as f32 * gain, 1e-3), "reversed {i}: {s}");
    }
}

#[test]
fn overlap_set_live_applies_from_the_next_hit() {
    // `Poly` (MAIN@1050) off: the next hit fast-fades the pad's own voices first; back on, voices overlap again.
    let mut sampler = PadSampler::new(vec![pad(dc(100_000))], RATE);
    let gain = level_gain(0.8531);
    sampler.set_overlap(0, false);
    sampler.hit(0, 1.0, 0.5, 1.0);
    left(&mut sampler, 0, 100);
    sampler.hit(0, 1.0, 0.5, 1.0);
    assert!(close(left(&mut sampler, 0, 3000)[2999], gain, 1e-4));
    sampler.set_overlap(0, true);
    sampler.hit(0, 1.0, 0.5, 1.0);
    assert!(close(left(&mut sampler, 0, 3000)[2999], 2.0 * gain, 1e-3));
}

#[test]
fn a_stretch_streamed_in_pieces_renders_what_the_whole_stretch_renders() {
    // The first 8192 frames at once, the rest streamed in.
    let source = SampleData {
        right: Some(sine(330.0, 30_011).left),
        ..sine(220.0, 30_011)
    };
    for (ratio, semitones) in [(2.0, 0.0), (0.5, 3.0), (1.37, -5.0), (1.0, 7.0)] {
        let whole = stretch(&source, ratio, semitones);
        let mut render = Stretching::new(&source, ratio, semitones);
        let stream = render.stream().clone();
        assert_eq!(stream.frames(), whole.frames());
        render.fill(STREAM_FRAMES);
        assert_eq!(stream.filled(), STREAM_FRAMES.min(whole.frames()));
        for upto in [9000, 9001, 12_345, 40_000] {
            render.fill(upto);
            assert_eq!(stream.filled(), upto.min(whole.frames()));
        }
        assert_eq!(render.finish(), whole, "x{ratio} {semitones} st");
    }
    // Both channels render alike: a pitched stereo source whose right is its left renders a right that is its left.
    let twin = SampleData {
        right: Some(source.left.clone()),
        ..source.clone()
    };
    let whole = stretch(&twin, 1.37, -5.0);
    assert_eq!(whole.right, Some(whole.left.clone()));
    // Nothing to render (the semitones round to 0): the source, all of it at once.
    let same = Stretching::new(&source, 1.0, 0.3);
    assert_eq!(same.stream().filled(), source.frames());
    assert_eq!(same.finish(), source);
}

#[test]
fn a_streaming_pad_plays_the_frames_written_then_silence_and_later_frames_as_they_arrive() {
    // The pad plays the new, partly filled buffer at once. U
    // ruled: a voice that reaches frames not yet written reads silence there and plays on.
    let stream = Arc::new(Stream::new(RATE as u32, 20_000, true));
    let ramp: Vec<f32> = (0..20_000).map(|i| i as f32).collect();
    let down: Vec<f32> = ramp.iter().map(|x| -x).collect();
    stream.push(&ramp[..STREAM_FRAMES], Some(&down[..STREAM_FRAMES]));
    let streaming = Pad {
        sample: Some(PadSample::Streaming(stream.clone())),
        ..Pad::default()
    };
    let mut sampler = PadSampler::new(vec![streaming], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    let gain = level_gain(0.8531);
    let (l, r) = render(&mut sampler, 0, 10_000);
    for i in [1, 100, 5000, STREAM_FRAMES - 1] {
        assert!(close(l[i] / gain, i as f32, 1e-2), "{i}: {}", l[i]);
        assert!(close(r[i] / gain, -(i as f32), 1e-2), "{i}: {}", r[i]);
    }
    assert!(
        (l[STREAM_FRAMES..].iter().chain(&r[STREAM_FRAMES..])).all(|&s| s == 0.0),
        "silence past the fill"
    );
    assert_eq!(sampler.active_voices(0), 1, "it plays on");
    // The rest arrives: the same voice reads it from its frame.
    stream.push(&ramp[STREAM_FRAMES..], Some(&down[STREAM_FRAMES..]));
    assert_eq!(stream.filled(), 20_000);
    let (l, r) = render(&mut sampler, 0, 100);
    assert!(close(l[0] / gain, 10_000.0, 1e-1) && close(r[0] / gain, -10_000.0, 1e-1));
    // Reversed, a voice starts at the end, which streams in last: silent until the tail arrives.
    let tail = Arc::new(Stream::new(RATE as u32, 20_000, false));
    tail.push(&ramp[..STREAM_FRAMES], None);
    let reversed = Pad {
        sample: Some(PadSample::Streaming(tail.clone())),
        reverse: true,
        ..Pad::default()
    };
    let mut sampler = PadSampler::new(vec![reversed], RATE);
    sampler.hit(0, 1.0, 0.5, 1.0);
    assert!(left(&mut sampler, 0, 1000).iter().all(|&s| s == 0.0));
    tail.push(&ramp[STREAM_FRAMES..], None);
    let l = left(&mut sampler, 0, 10);
    assert!(
        close(l[0] / gain, (20_000 - 1 - 1000) as f32, 1e-1),
        "{}",
        l[0]
    );
}

/// A pad whose sample goes away (an edit's engine has none for it) ends the voices it takes
/// over, rather than holding them frozen for the next edit that gives it a sample back to resume.
#[test]
fn a_pad_without_a_sample_ends_the_voices_it_takes_over() {
    let mut old = PadSampler::new(vec![pad(dc(100_000))], RATE);
    old.hit(0, 1.0, 0.5, 1.0);
    left(&mut old, 0, 256);
    let mut gone = PadSampler::new(vec![Pad::default()], RATE);
    gone.adopt(&old, Some);
    assert!(left(&mut gone, 0, 256).iter().all(|&s| s == 0.0));
    let mut back = PadSampler::new(vec![pad(dc(100_000))], RATE);
    back.adopt(&gone, Some);
    assert_eq!(back.active_voices(0), 0);
    assert!(left(&mut back, 0, 256).iter().all(|&s| s == 0.0));
}
