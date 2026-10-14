use honeydew_audio::sample_edit::{
    SUMMARY_FRAMES, clear, copy_to, delete_space, duplicate, fade_in, fade_out, frame_at,
    insert_silence, move_to, normalize, peaks, speed_ratio, speed_semitones, summarize, trim,
};

/// A stereo buffer: left 1, 2, 3…, right the negatives.
fn stereo(n: usize) -> Vec<Vec<f32>> {
    let left: Vec<f32> = (1..=n).map(|i| i as f32).collect();
    let right = left.iter().map(|x| -x).collect();
    vec![left, right]
}

fn left(ch: &[Vec<f32>]) -> Vec<f32> {
    assert!(
        ch.iter().all(|c| c.len() == ch[0].len()),
        "channels differ in length"
    );
    assert!(
        ch[1].iter().zip(&ch[0]).all(|(r, l)| *r == -l),
        "right is not -left"
    );
    ch[0].clone()
}

#[test]
fn frame_at_truncates_then_scales_by_speed() {
    // 120 BPM at 44.1 kHz: 22 050 frames per beat.
    assert_eq!(frame_at(0.0, 44_100, 120.0, 1.0), 0);
    assert_eq!(frame_at(1.0, 44_100, 120.0, 1.0), 22_050);
    assert_eq!(frame_at(1.5, 44_100, 120.0, 1.0), 33_075);
    // trunc(beat·60·rate/tempo) before the speed: 0.0000726 beats = 1.6 frames → 1, × 2 = 2 (3 without the trunc).
    assert_eq!(frame_at(0.000_072_6, 44_100, 120.0, 2.0), 2);
    assert_eq!(frame_at(1.0, 44_100, 120.0, 2.0), 44_100);
    assert_eq!(frame_at(1.0, 44_100, 120.0, 0.5), 11_025);
    assert_eq!(frame_at(-1.0, 44_100, 120.0, 1.0), 0);
}

#[test]
fn a_reversed_sample_is_edited_by_reversing_around_the_op() {
    // Displayed reversed as 6 5 4 3 2 1; `Duplicate` over display frames [1, 3) shows 6 5 4 5 4 3 2 1.
    let mut ch = stereo(6);
    ch.iter_mut().for_each(|c| c.reverse());
    duplicate(
        &mut ch,
        frame_at(1.0, 1, 60.0, 1.0)..frame_at(3.0, 1, 60.0, 1.0),
    );
    ch.iter_mut().for_each(|c| c.reverse());
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 4.0, 5.0, 4.0, 5.0, 6.0]);
}

/// s = trunc(fma_f32(v, 48, −24)) at v = f32(k/48) is k − 24 except at these k.
const FUSED: [(u32, i32); 9] = [
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

/// The semitones Speed plays at the stepped value v = f32(k/48), k = 0..=48.
fn stepped_semitones(k: u32) -> i32 {
    FUSED
        .iter()
        .find(|(at, _)| *at == k)
        .map_or(k as i32 - 24, |(_, s)| *s)
}

#[test]
fn speed_semitones_follow_the_fused_law_on_every_step() {
    for k in 0..=48u32 {
        let v = k as f32 / 48.0;
        assert_eq!(speed_semitones(v), stepped_semitones(k), "k = {k}");
    }
    // The odd steps by name: a plain multiply-then-subtract would give k − 24 there.
    assert_eq!(speed_semitones(25.0 / 48.0), 0);
    assert_eq!(speed_semitones(16.0 / 48.0), -7);
    assert_eq!(speed_semitones(40.0 / 48.0), 15);
    assert_eq!(speed_semitones(0.0), -24);
    assert_eq!(speed_semitones(1.0), 24);
}

#[test]
fn speed_ratio_is_exp2f_of_s_times_f32_one_twelfth() {
    let twelfth = f32::from_bits(0x3DAA_AAAB);
    for k in 0..=48u32 {
        let v = k as f32 / 48.0;
        let s = stepped_semitones(k) as f32;
        assert_eq!(speed_ratio(v), f64::from((s * twelfth).exp2()), "k = {k}");
    }
    assert_eq!(speed_ratio(0.5), 1.0);
}

#[test]
fn insert_silence_adds_zero_frames_at_the_position() {
    let mut ch = stereo(4);
    insert_silence(&mut ch, 1, 2);
    assert_eq!(left(&ch), [1.0, 0.0, 0.0, 2.0, 3.0, 4.0]);
    insert_silence(&mut ch, 6, 1);
    assert_eq!(left(&ch), [1.0, 0.0, 0.0, 2.0, 3.0, 4.0, 0.0]);
    // Past the end: the silence before it is materialised.
    let mut ch = stereo(2);
    insert_silence(&mut ch, 4, 1);
    assert_eq!(left(&ch), [1.0, 2.0, 0.0, 0.0, 0.0]);
    let mut ch = stereo(2);
    insert_silence(&mut ch, 1, 0);
    assert_eq!(left(&ch), [1.0, 2.0]);
}

#[test]
fn clear_zeroes_the_range_and_keeps_the_length() {
    let mut ch = stereo(5);
    clear(&mut ch, 1..3);
    assert_eq!(left(&ch), [1.0, 0.0, 0.0, 4.0, 5.0]);
    clear(&mut ch, 4..9);
    assert_eq!(left(&ch), [1.0, 0.0, 0.0, 4.0, 0.0]);
    clear(&mut ch, 7..9);
    assert_eq!(left(&ch), [1.0, 0.0, 0.0, 4.0, 0.0]);
}

#[test]
fn delete_space_removes_the_range() {
    let mut ch = stereo(5);
    delete_space(&mut ch, 1..3);
    assert_eq!(left(&ch), [1.0, 4.0, 5.0]);
    delete_space(&mut ch, 2..10);
    assert_eq!(left(&ch), [1.0, 4.0]);
    delete_space(&mut ch, 5..10);
    assert_eq!(left(&ch), [1.0, 4.0]);
    delete_space(&mut ch, 1..1);
    assert_eq!(left(&ch), [1.0, 4.0]);
}

#[test]
fn trim_keeps_only_the_range() {
    let mut ch = stereo(6);
    trim(&mut ch, 2..4);
    assert_eq!(left(&ch), [3.0, 4.0]);
    let mut ch = stereo(4);
    trim(&mut ch, 1..9);
    assert_eq!(left(&ch), [2.0, 3.0, 4.0]);
    let mut ch = stereo(4);
    trim(&mut ch, 6..9);
    assert_eq!(left(&ch), [] as [f32; 0]);
}

#[test]
fn duplicate_copies_the_range_right_after_it() {
    let mut ch = stereo(5);
    duplicate(&mut ch, 1..3);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 5.0]);
    // A range running past the end copies its silence too, so the copy still lands at e.
    let mut ch = stereo(3);
    duplicate(&mut ch, 2..4);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 0.0, 3.0, 0.0]);
}

#[test]
fn copy_to_playhead_inserts_the_range_at_p_only_from_outside_it() {
    // p after the range.
    let mut ch = stereo(6);
    copy_to(&mut ch, 1..3, 5);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 4.0, 5.0, 2.0, 3.0, 6.0]);
    // p before the range.
    let mut ch = stereo(6);
    copy_to(&mut ch, 3..5, 1);
    assert_eq!(left(&ch), [1.0, 4.0, 5.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    // p on either edge counts as outside.
    let mut ch = stereo(4);
    copy_to(&mut ch, 1..3, 1);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 2.0, 3.0, 4.0]);
    let mut ch = stereo(4);
    copy_to(&mut ch, 1..3, 3);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 2.0, 3.0, 4.0]);
    // p inside: nothing.
    let mut ch = stereo(4);
    copy_to(&mut ch, 1..3, 2);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 4.0]);
    // p past the end: the copy lands at p.
    let mut ch = stereo(3);
    copy_to(&mut ch, 0..2, 5);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 2.0]);
    // A range running past the end, p before it: its silence is copied too.
    let mut ch = stereo(3);
    copy_to(&mut ch, 1..5, 0);
    assert_eq!(left(&ch), [2.0, 3.0, 0.0, 0.0, 1.0, 2.0, 3.0, 0.0, 0.0]);
}

#[test]
fn move_to_playhead_copies_then_closes_the_source() {
    let mut ch = stereo(6);
    move_to(&mut ch, 1..3, 5);
    assert_eq!(left(&ch), [1.0, 4.0, 5.0, 2.0, 3.0, 6.0]);
    let mut ch = stereo(6);
    move_to(&mut ch, 3..5, 1);
    assert_eq!(left(&ch), [1.0, 4.0, 5.0, 2.0, 3.0, 6.0]);
    // On an edge: no net change.
    for p in [1, 3] {
        let mut ch = stereo(4);
        move_to(&mut ch, 1..3, p);
        assert_eq!(left(&ch), [1.0, 2.0, 3.0, 4.0]);
    }
    // p inside: nothing.
    let mut ch = stereo(4);
    move_to(&mut ch, 0..3, 1);
    assert_eq!(left(&ch), [1.0, 2.0, 3.0, 4.0]);
    // p past the end.
    let mut ch = stereo(3);
    move_to(&mut ch, 0..1, 4);
    assert_eq!(left(&ch), [2.0, 3.0, 0.0, 1.0]);
    // A range running past the end, p before it: the moved source is removed in full.
    let mut ch = stereo(3);
    move_to(&mut ch, 1..5, 0);
    assert_eq!(left(&ch), [2.0, 3.0, 0.0, 0.0, 1.0]);
}

#[test]
fn normalize_scales_the_range_by_one_over_the_peak_of_both_channels() {
    let mut ch = vec![vec![0.1, 0.2, -0.1, 0.9], vec![0.05, -0.1, 0.4, 0.0]];
    normalize(&mut ch, 0..3);
    assert_eq!(ch[0], [0.1 / 0.4, 0.2 / 0.4, -0.1 / 0.4, 0.9]);
    assert_eq!(ch[1], [0.05 / 0.4, -0.1 / 0.4, 1.0, 0.0]);
    // The peak lands exactly on 1.0, negative too.
    let mut ch = vec![vec![0.0, -0.3, 0.1]];
    normalize(&mut ch, 0..3);
    assert_eq!(ch[0], [0.0, -1.0, 0.1 / 0.3]);
    // Silent range: untouched.
    let mut ch = vec![vec![0.5, 0.0, 0.0, 0.5]];
    normalize(&mut ch, 1..3);
    assert_eq!(ch[0], [0.5, 0.0, 0.0, 0.5]);
    // Past the end: only the frames present count.
    let mut ch = vec![vec![0.5, 0.25]];
    normalize(&mut ch, 1..9);
    assert_eq!(ch[0], [0.5, 1.0]);
}

#[test]
fn fades_are_linear_k_over_n() {
    let mut ch = vec![vec![1.0; 6], vec![-1.0; 6]];
    fade_in(&mut ch, 1..5);
    assert_eq!(ch[0], [1.0, 0.0, 0.25, 0.5, 0.75, 1.0]);
    assert_eq!(ch[1], [-1.0, -0.0, -0.25, -0.5, -0.75, -1.0]);
    let mut ch = vec![vec![1.0; 6]];
    fade_out(&mut ch, 1..5);
    assert_eq!(ch[0], [1.0, 1.0, 0.75, 0.5, 0.25, 1.0]);
    // A range past the end keeps its N: the frames present get the same gains.
    let mut ch = vec![vec![1.0; 3]];
    fade_in(&mut ch, 1..5);
    assert_eq!(ch[0], [1.0, 0.0, 0.25]);
    let mut ch = vec![vec![1.0; 3]];
    fade_out(&mut ch, 1..5);
    assert_eq!(ch[0], [1.0, 1.0, 0.75]);
}

#[test]
fn peaks_of_a_ramp() {
    let ramp: Vec<f32> = (0..10).map(|i| i as f32).collect();
    // Two frames per pixel.
    assert_eq!(
        peaks(&ramp, &[], 2.0, 0.0, 5),
        [(0.0, 1.0), (2.0, 3.0), (4.0, 5.0), (6.0, 7.0), (8.0, 9.0)]
    );
    // Columns cover the frames whose positions fall in [x·fpp, (x+1)·fpp).
    assert_eq!(
        peaks(&ramp, &[], 2.5, 0.0, 4),
        [(0.0, 2.0), (3.0, 4.0), (5.0, 7.0), (8.0, 9.0)]
    );
    // A scroll offset, and columns past the end are (0, 0).
    assert_eq!(
        peaks(&ramp, &[], 3.0, 4.0, 3),
        [(4.0, 6.0), (7.0, 9.0), (0.0, 0.0)]
    );
    // Narrower than a frame: the frame under the column.
    assert_eq!(
        peaks(&ramp, &[], 0.5, 1.0, 4),
        [(1.0, 1.0), (1.0, 1.0), (2.0, 2.0), (2.0, 2.0)]
    );
    // Before the start (a mirrored view can begin there).
    assert_eq!(
        peaks(&[1.0, 2.0, 3.0], &[], 2.0, -3.0, 3),
        [(0.0, 0.0), (1.0, 1.0), (2.0, 3.0)]
    );
    // Min and max of a falling, sign-changing column.
    let wave = [0.5f32, -0.25, 0.75, -1.0];
    assert_eq!(peaks(&wave, &[], 4.0, 0.0, 1), [(-1.0, 0.75)]);
}

#[test]
fn peaks_read_the_whole_blocks_from_the_summary_and_equal_the_frames() {
    let x: Vec<f32> = (0..10_000)
        .map(|i| ((i * 7919) % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    let summary = summarize(&x);
    assert_eq!(summary.len(), 10_000usize.div_ceil(SUMMARY_FRAMES));
    assert_eq!(summary[0], summarize(&x[..SUMMARY_FRAMES])[0]);
    for (fpp, first, width) in [
        (1000.0, 0.0, 10),
        (333.3, 17.5, 30),
        (2.0, 5.0, 40),
        (5000.0, -2500.0, 3),
        (700.0, 9500.0, 3),
    ] {
        let raw = peaks(&x, &[], fpp, first, width);
        assert_eq!(peaks(&x, &summary, fpp, first, width), raw, "{fpp} {first}");
    }
    // The summary path: silent frames under a summary saying ±0.5 read ±0.5 where a column covers a whole block, and
    // their own frames where it covers none.
    let silent = vec![0.0; 4 * SUMMARY_FRAMES];
    let loud = vec![(-0.5, 0.5); 4];
    assert_eq!(peaks(&silent, &loud, 1024.0, 0.0, 1), [(-0.5, 0.5)]);
    assert_eq!(peaks(&silent, &loud, 100.0, 0.0, 2), [(0.0, 0.0); 2]);
    assert!(summarize(&[]).is_empty());
}

#[test]
fn frame_at_clamps_negative_and_non_finite_input_to_frame_0() {
    // A negative beat or speed (two negatives made a positive frame), NaN, infinity and a tempo of 0 or less.
    let f = |beat: f64, tempo: f64, speed: f64| frame_at(beat, 44_100, tempo, speed);
    assert_eq!(f(-1.0, 120.0, -1.0), 0);
    assert_eq!(f(1.0, 120.0, -1.0), 0);
    assert_eq!(f(-1.0, 120.0, 1.0), 0);
    assert_eq!(f(1.0, -120.0, 1.0), 0);
    assert_eq!(f(-1.0, -120.0, 1.0), 0);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(f(bad, 120.0, 1.0), 0, "beat {bad}");
        assert_eq!(f(1.0, bad, 1.0), 0, "tempo {bad}");
        assert_eq!(f(1.0, 120.0, bad), 0, "speed {bad}");
    }
    assert_eq!(f(1.0, 0.0, 1.0), 0, "tempo 0");
    assert_eq!(f(0.0, 0.0, 1.0), 0, "0 / 0");
    assert_eq!(f(1.0, 120.0, 0.0), 0, "speed 0");
}
