//! The six built-in Slicer loops: drum patterns rendered from the factory kits, with the tempo and one
//! slice marker per hit step.

use honeydew_audio::builtin_drums::{KITS, RATE, builtin};
use honeydew_audio::builtin_loops::{LOOPS, Loop, loop_sample};
use honeydew_audio::sample::SampleData;
use honeydew_audio::slicer::{MAX_SLICES, auto_clip_beats, detect};

const MINUS_1_DBFS: f32 = 0.891_250_9;
/// The 5 ms fade, in frames.
const FADE: usize = RATE as usize / 200;

const NAMES: [&str; 6] = [
    "Pocket Groove 92",
    "Dust Break 84",
    "Relay Bounce 100",
    "Relay Roll 140",
    "Foundry Drive 128",
    "Foundry Rush 172",
];

fn render(name: &str) -> SampleData {
    loop_sample(name).unwrap_or_else(|| panic!("no loop {name}"))
}

/// A row's steps with the spaces dropped.
fn steps(row: &str) -> Vec<char> {
    row.chars().filter(|c| *c != ' ').collect()
}

/// The steps (0-based, over all bars) that any row hits, ascending.
fn hit_steps(l: &Loop) -> Vec<usize> {
    let mut hits: Vec<usize> = l
        .rows
        .iter()
        .flat_map(|(_, row)| {
            steps(row)
                .into_iter()
                .enumerate()
                .filter(|(_, c)| matches!(c, 'x' | 'X'))
                .map(|(i, _)| i)
        })
        .collect();
    hits.sort_unstable();
    hits.dedup();
    hits
}

/// The frame a step starts on: the loop's frames split into `16 * bars` equal steps.
fn step_frame(l: &Loop, step: usize) -> usize {
    let frames = (f64::from(l.bars) * 240.0 * f64::from(RATE) / f64::from(l.bpm)).round();
    (step as f64 * frames / f64::from(16 * l.bars)).round() as usize
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// A one-bar loop of 16 steps at 120 bpm (88200 frames), for the mixing rules.
fn custom(kit: &'static str, rows: &'static [(&'static str, &'static str)]) -> SampleData {
    Loop {
        name: "Test",
        kit,
        bpm: 120,
        bars: 1,
        rows,
    }
    .render()
    .expect("the test loop renders")
}

#[test]
fn six_named_loops_each_on_a_factory_kit() {
    let names: Vec<&str> = LOOPS.iter().map(|l| l.name).collect();
    assert_eq!(names, NAMES);
    for l in &LOOPS {
        let (_, pads) = KITS
            .iter()
            .find(|(key, _)| *key == l.kit)
            .unwrap_or_else(|| panic!("{}: no kit {}", l.name, l.kit));
        assert!(l.name.ends_with(&format!(" {}", l.bpm)), "{}", l.name);
        assert!((1..=4).contains(&l.bars), "{}: {} bars", l.name, l.bars);
        for (pad, row) in l.rows {
            assert!(pads.contains(pad), "{}: {} has no pad {pad}", l.name, l.kit);
            assert!(
                builtin(&format!("Built-in/{}/{pad}", l.kit)).is_some(),
                "{}: {pad}",
                l.name
            );
            let row = steps(row);
            assert_eq!(row.len(), 16 * l.bars as usize, "{} {pad}", l.name);
            assert!(row.iter().all(|c| ".xX".contains(*c)), "{} {pad}", l.name);
        }
        let pads_used: Vec<&str> = l.rows.iter().map(|(pad, _)| *pad).collect();
        for (i, pad) in pads_used.iter().enumerate() {
            assert!(!pads_used[..i].contains(pad), "{}: {pad} twice", l.name);
        }
        let hits = hit_steps(l).len();
        assert!(
            (8..=MAX_SLICES).contains(&hits),
            "{}: {hits} hit steps",
            l.name
        );
    }
}

#[test]
fn each_loop_is_exactly_its_bars_long() {
    for l in &LOOPS {
        let sample = render(l.name);
        let seconds = f64::from(l.bars) * 4.0 * 60.0 / f64::from(l.bpm);
        let frames = sample.frames();
        assert!(
            (frames as f64 - seconds * f64::from(RATE)).abs() <= 0.5,
            "{}: {frames} frames for {seconds} s",
            l.name
        );
        assert_eq!(sample.sample_rate, RATE);
        assert!(sample.right.is_none(), "{} is mono", l.name);
    }
}

#[test]
fn peak_is_minus_1_dbfs_and_the_end_is_silent_and_it_is_deterministic() {
    for l in &LOOPS {
        let sample = render(l.name);
        assert!(sample.left.iter().all(|s| s.is_finite()), "{}", l.name);
        let p = peak(&sample.left);
        assert!((p - MINUS_1_DBFS).abs() < 1e-4, "{}: peak {p}", l.name);
        assert_eq!(*sample.left.last().unwrap(), 0.0, "{}", l.name);
        assert_eq!(sample, render(l.name), "{}: two renders differ", l.name);
    }
}

#[test]
fn the_loops_differ_and_none_is_quiet() {
    for (i, a) in LOOPS.iter().enumerate() {
        let x = render(a.name);
        assert!(rms(&x.left) > 0.03, "{}: rms {}", a.name, rms(&x.left));
        for b in &LOOPS[i + 1..] {
            assert_ne!(x.left, render(b.name).left, "{} = {}", a.name, b.name);
        }
    }
}

#[test]
fn tempo_and_one_cue_per_hit_step() {
    for l in &LOOPS {
        let sample = render(l.name);
        assert_eq!(sample.meta.tempo, Some(l.bpm as f32), "{}", l.name);
        let expected: Vec<u32> = hit_steps(l)
            .into_iter()
            .map(|s| step_frame(l, s) as u32)
            .collect();
        assert_eq!(sample.meta.cues, expected, "{}", l.name);
        assert_eq!(sample.meta.cues[0], 0, "{}: a hit on step 0", l.name);
    }
}

#[test]
fn a_hit_starts_on_its_step_and_nothing_sounds_before_it() {
    // Steps 4 and 8 of a bar at 120 bpm start at frames 22050 and 44100.
    let x = custom("Pocket", &[("Kick", "....X...x.......")]);
    assert_eq!(x.meta.cues, [22_050, 44_100]);
    assert!(x.left[..22_050].iter().all(|s| *s == 0.0));
    let ten_ms = RATE as usize / 100;
    assert!(rms(&x.left[22_050..22_050 + ten_ms]) > 0.1);
    assert!(rms(&x.left[44_100..44_100 + ten_ms]) > 0.05);
}

#[test]
fn slicer_takes_the_cues_as_the_slices() {
    for l in &LOOPS {
        let sample = render(l.name);
        let cues: Vec<usize> = sample.meta.cues.iter().map(|&c| c as usize).collect();
        assert_eq!(detect(&sample, 0.0), cues, "{}", l.name);
    }
}

#[test]
fn onset_detection_finds_at_least_half_the_hit_steps_without_the_cues() {
    // A detected start counts when it is within 30 ms of the step.
    let window = RATE as usize * 3 / 100;
    for l in &LOOPS {
        let mut sample = render(l.name);
        sample.meta.cues.clear();
        let found = detect(&sample, 0.25);
        let steps = hit_steps(l);
        let hit = steps
            .iter()
            .filter(|&&s| {
                let at = step_frame(l, s);
                found.iter().any(|&f| f.abs_diff(at) <= window)
            })
            .count();
        assert!(
            2 * hit >= steps.len(),
            "{}: {hit} of {} hit steps found ({found:?})",
            l.name,
            steps.len()
        );
    }
}

#[test]
fn auto_clip_is_the_loops_bars_in_beats() {
    for l in &LOOPS {
        let sample = render(l.name);
        // The song tempo plays no part when the loop carries its own.
        for song_bpm in [60.0, 120.0, 200.0] {
            let beats = auto_clip_beats(sample.frames(), sample.meta.tempo, song_bpm);
            assert!(
                (beats - f64::from(l.bars) * 4.0).abs() < 1e-3,
                "{}: {beats} beats at song {song_bpm}",
                l.name
            );
        }
    }
}

#[test]
fn the_dispatch_takes_either_separator_and_only_known_names() {
    for l in &LOOPS {
        let slash = builtin(&format!("Built-in/Loops/{}", l.name)).unwrap();
        let backslash = builtin(&format!("Built-in\\Loops\\{}", l.name)).unwrap();
        assert_eq!(slash, render(l.name), "{}", l.name);
        assert_eq!(slash, backslash, "{}", l.name);
    }
    for folder in [
        "Built-in/Loops",
        "Built-in/Loops/",
        "Built-in/Loops/Nope 100",
        "Built-in/Loops/pocket groove 92",
        "Built-in/Loops/Pocket Groove 92.wav",
        "Built-in/Loops/Pocket Groove 92/x",
        "Loops/Pocket Groove 92",
    ] {
        assert!(builtin(folder).is_none(), "{folder:?}");
    }
    assert!(loop_sample("").is_none());
}

#[test]
fn an_unknown_pad_or_kit_does_not_render() {
    let render = |kit, rows| {
        Loop {
            name: "Test",
            kit,
            bpm: 120,
            bars: 1,
            rows,
        }
        .render()
    };
    assert!(render("Pocket", &[("Kick", "x...............")]).is_some());
    assert!(render("Pocket", &[("Cowbell", "x...............")]).is_none());
    assert!(render("Nowhere", &[("Kick", "x...............")]).is_none());
    // A row of the wrong length is not guessed at.
    assert!(render("Pocket", &[("Kick", "x...")]).is_none());
}

#[test]
fn an_accent_is_louder_than_a_plain_hit() {
    let x = custom("Pocket", &[("Kick", "X... .... x... ....")]);
    let (accent, plain) = (peak(&x.left[..22_050]), peak(&x.left[44_100..66_150]));
    assert!(accent > 1.4 * plain, "accent {accent}, plain {plain}");
    assert!(
        (accent - MINUS_1_DBFS).abs() < 1e-4,
        "the loudest is −1 dBFS"
    );
}

#[test]
fn a_hit_is_cut_by_the_next_hit_of_its_pad() {
    // A relay kick rings 1.2 s. The second kick, 4 steps (frame 22050) on, takes over and cuts the first with a
    // 5 ms fade. All three loops peak on one kick's own peak, so they share a gain.
    let cut = 22_050;
    let two = custom("Relay", &[("Kick", "X... X...........")]);
    let first = custom("Relay", &[("Kick", "X...............")]);
    let second = custom("Relay", &[("Kick", "....X...........")]);
    assert_eq!(two.left[..cut - FADE], first.left[..cut - FADE]);
    for i in 0..FADE {
        let fade = (FADE - 1 - i) as f32 / (FADE - 1) as f32;
        let (at, alone) = (two.left[cut - FADE + i], first.left[cut - FADE + i]);
        assert!((at - alone * fade).abs() < 1e-6, "{i}: {at} for {alone}");
    }
    assert!(
        peak(&first.left[cut..cut + 2205]) > 0.01,
        "uncut it rings on"
    );
    assert_eq!(two.left[cut..], second.left[cut..]);
}

#[test]
fn a_closed_hat_chokes_the_open_hat() {
    // Dust's open hat rings 1.6 s; a closed hat on step 4 (frame 22050) stops it. Both loops peak on the open hat.
    let open = custom("Dust", &[("Open Hat", "X...............")]);
    let both = custom(
        "Dust",
        &[
            ("Open Hat", "X..............."),
            ("Closed Hat", "....x..........."),
        ],
    );
    // A quarter second on, the closed hat's own tail is gone.
    let later = 22_050 + RATE as usize / 4..22_050 + RATE as usize * 7 / 20;
    let (ringing, choked) = (rms(&open.left[later.clone()]), rms(&both.left[later]));
    assert!(ringing > 0.003, "the open hat alone rings: {ringing}");
    assert!(choked < ringing / 20.0, "{choked} against {ringing}");
    // Up to the closed hat they are the same.
    assert_eq!(open.left[..22_050 - FADE], both.left[..22_050 - FADE]);
}

#[test]
fn a_hit_ringing_past_the_end_is_cut_there_with_a_fade() {
    // Foundry's crash rings 3 s; on the last step of a 2 s loop it is cut at the end.
    let x = custom("Foundry", &[("Crash", "...............X")]);
    let frames = x.frames();
    assert_eq!(frames, 88_200);
    assert_eq!(x.left[frames - 1], 0.0);
    assert!(
        rms(&x.left[frames - 4 * FADE..frames - 2 * FADE]) > 0.1,
        "full level before the fade"
    );
    // A linear ramp: the first half of the fade has about three times the amplitude of the second.
    let (first, second) = (
        rms(&x.left[frames - FADE..frames - FADE / 2]),
        rms(&x.left[frames - FADE / 2..]),
    );
    assert!(first > 2.0 * second, "{first} then {second}");
}
