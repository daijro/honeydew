// The module uses `crate::biquad::flush`.
#[allow(dead_code)]
#[path = "../src/biquad.rs"]
mod biquad;
#[path = "../src/reverbs.rs"]
mod reverbs;

use reverbs::ReverbState;

fn defaults(id: u16) -> Vec<f32> {
    match id {
        12 => vec![
            1.0, 0.4, 0.8, 0.4, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.499, 0.0, 0.0,
        ],
        32 => vec![
            1.0, 0.4, 0.8, 0.4, 0.5, 0.25, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.499, 0.0, 1.0, 1.0,
        ],
        35 => vec![
            1.0, 0.16, 0.0187, 0.5, 0.068, 1.0, 0.16, 0.8, 0.8, 0.56, 0.0, 0.0, 0.0, 0.0, 0.28,
            0.5, 0.33, 0.2405, 0.5,
        ],
        _ => panic!("invalid reverb id"),
    }
}

fn render(id: u16, params: &[f32], frames: usize, right_impulse: bool) -> (Vec<f32>, Vec<f32>) {
    let mut reverb = ReverbState::new(id, params, 48_000.0);
    reverb.prepare_block(120.0);
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    for i in 0..frames {
        (left[i], right[i]) = reverb.process_frame(
            if i == 0 && !right_impulse { 1.0 } else { 0.0 },
            if i == 0 && right_impulse { 1.0 } else { 0.0 },
        );
    }
    (left, right)
}

#[test]
fn each_reverb_keeps_an_impulse_tail_after_input_stops() {
    for id in [12, 32, 35] {
        let (left, right) = render(id, &defaults(id), 24_000, false);
        assert!(left[2_000..].iter().any(|x| x.abs() > 1e-6), "ID {id}");
        assert!(right[2_000..].iter().any(|x| x.abs() > 1e-6), "ID {id}");
        assert!(left.iter().all(|x| x.is_finite()), "ID {id}");
        assert!(right.iter().all(|x| x.is_finite()), "ID {id}");
    }
}

#[test]
fn decay_control_changes_late_tail_energy() {
    for id in [12, 32, 35] {
        let mut short = defaults(id);
        let mut long = short.clone();
        let decay = if id == 35 { 4 } else { 1 };
        short[decay] = 0.02;
        long[decay] = if id == 35 { 0.45 } else { 0.9 };
        let (short_l, _) = render(id, &short, 48_000, false);
        let (long_l, _) = render(id, &long, 48_000, false);
        let energy = |samples: &[f32]| samples.iter().map(|x| x * x).sum::<f32>();
        assert!(
            energy(&long_l[24_000..]) > energy(&short_l[24_000..]) * 2.0,
            "ID {id}"
        );
    }
}

#[test]
fn reverb_two_predelay_responds_to_tempo() {
    let mut params = defaults(35);
    params[7] = 0.0;
    params[11] = 0.25;
    params[12] = 1.0;
    let mut fast = ReverbState::new(35, &params, 48_000.0);
    let mut slow = ReverbState::new(35, &params, 48_000.0);
    fast.prepare_block(240.0);
    slow.prepare_block(60.0);
    let mut first_fast = None;
    let mut first_slow = None;
    for i in 0..20_000 {
        let (fl, fr) = fast.process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0);
        let (sl, sr) = slow.process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0);
        if first_fast.is_none() && fl.abs().max(fr.abs()) > 1e-5 {
            first_fast = Some(i);
        }
        if first_slow.is_none() && sl.abs().max(sr.abs()) > 1e-5 {
            first_slow = Some(i);
        }
    }
    assert!(first_fast.unwrap() >= 3_000);
    assert!(first_slow.unwrap() > first_fast.unwrap() + 5_000);
}

#[test]
fn reverb_two_stereo_routing_distinguishes_impulse_sides() {
    let mut params = defaults(35);
    params[7] = 0.0;
    let left = render(35, &params, 12_000, false);
    let right = render(35, &params, 12_000, true);
    assert_ne!(left.0, right.0);
    assert!(left.1.iter().any(|x| x.abs() > 1e-5));
    assert!(right.0.iter().any(|x| x.abs() > 1e-5));
    params[15] = 0.0;
    let narrowed = render(35, &params, 12_000, false);
    assert_ne!(left.0, narrowed.0);
}

#[test]
fn reset_flushes_the_delay_network() {
    let params = defaults(32);
    let mut reverb = ReverbState::new(32, &params, 48_000.0);
    for i in 0..8_000 {
        reverb.process_frame(if i == 0 { 1.0 } else { 0.0 }, 0.0);
    }
    reverb.reset();
    for _ in 0..8_000 {
        let (l, r) = reverb.process_frame(0.0, 0.0);
        assert_eq!((l, r), (0.0, 0.0));
    }
}
